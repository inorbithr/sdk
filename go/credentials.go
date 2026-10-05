package inorbit

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"sync"
	"time"
	"unicode/utf8"
)

// The credential sources of docs/config.md section 5 as public types, so a program can
// build its own chain: CachedToken, TokenFile, CliToken and ChainedCredential, next to
// StaticToken and ClientCredentials.

const (
	// softExpiryRetry is how long a cached token that is still valid is used after a
	// failed refresh before the next refresh is tried.
	softExpiryRetry = 5 * time.Second

	// fileCheck is how often a token file's modification time and size are checked.
	fileCheck = 60 * time.Second

	// cliTimeout bounds one run of iohr auth token.
	cliTimeout = 10 * time.Second
)

// CachedToken gives any TokenProvider the caching rules of docs/config.md section 5.3:
// the token is kept in memory and refreshed when less than a fifth of its life is left;
// one refresh runs at a time, and concurrent callers wait for it (a caller that gives up
// does not cancel it for the others); when a refresh fails while the cached token is
// still valid, that token is used and the next refresh is tried no sooner than 5 s later.
// A token without an expiry is kept until Invalidate.
type CachedToken struct {
	source TokenProvider

	// onRefresh and onRefreshFailed let a client count exchanges and log soft expiry.
	onRefresh       func(err error)
	onRefreshFailed func(err error)

	mu      sync.Mutex
	held    *heldToken
	pending *refresh
	quiet   time.Time
}

type heldToken struct {
	token     Token
	refreshAt time.Time // the zero time: never
}

type refresh struct {
	done  chan struct{}
	token *heldToken
	err   error
}

// NewCachedToken caches what source hands out.
func NewCachedToken(source TokenProvider) *CachedToken { return &CachedToken{source: source} }

// String never shows the token.
func (*CachedToken) String() string { return "CachedToken(" + redacted + ")" }

// GoString never shows the token.
func (c *CachedToken) GoString() string { return c.String() }

func (h *heldToken) fresh(now time.Time) bool {
	return h.refreshAt.IsZero() || now.Before(h.refreshAt)
}

func (h *heldToken) valid(now time.Time) bool {
	return !h.token.ExpiresAt.IsZero() && now.Before(h.token.ExpiresAt)
}

// Token returns the cached token while four fifths of its life are left, else a fresh
// one.
func (c *CachedToken) Token(ctx context.Context) (Token, error) {
	c.mu.Lock()
	now := time.Now()
	if h := c.held; h != nil && (h.fresh(now) || h.valid(now) && now.Before(c.quiet)) {
		c.mu.Unlock()
		return h.token, nil
	}
	r := c.pending
	if r == nil {
		r = &refresh{done: make(chan struct{})}
		c.pending = r
		go c.fetch(context.WithoutCancel(ctx), r)
	}
	c.mu.Unlock()
	select {
	case <-ctx.Done():
		return Token{}, ctx.Err()
	case <-r.done:
	}
	if r.err == nil {
		return r.token.token, nil
	}
	c.mu.Lock()
	h := c.held
	if h != nil && h.valid(time.Now()) {
		c.quiet = time.Now().Add(softExpiryRetry)
		c.mu.Unlock()
		if c.onRefreshFailed != nil {
			c.onRefreshFailed(r.err)
		}
		return h.token, nil
	}
	c.mu.Unlock()
	return Token{}, r.err
}

// fetch runs one refresh; it outlives a caller that gives up, so the others still get it.
func (c *CachedToken) fetch(ctx context.Context, r *refresh) {
	t, err := c.source.Token(ctx)
	if c.onRefresh != nil {
		c.onRefresh(err)
	}
	c.mu.Lock()
	if err == nil {
		now := time.Now()
		h := &heldToken{token: t}
		if !t.ExpiresAt.IsZero() {
			h.refreshAt = now.Add(max(t.ExpiresAt.Sub(now), 0) * 4 / 5)
		}
		c.held, r.token = h, h
	}
	r.err = err
	c.pending = nil
	c.mu.Unlock()
	close(r.done)
}

// Invalidate drops the cached token and tells the source, so the next call fetches a
// fresh one.
func (c *CachedToken) Invalidate() {
	c.mu.Lock()
	c.held = nil
	c.quiet = time.Time{}
	c.mu.Unlock()
	if inv, ok := c.source.(Invalidator); ok {
		inv.Invalidate()
	}
}

// TokenFile is a bearer token read from a file, such as a mounted Kubernetes Secret:
// read at first use, again when the file's modification time or size changes (checked
// at most once a minute), and at once after the API refused the token. When the content
// is a JWT, its exp claim (read, not verified) is the token's expiry. When the file
// disappears, the token read last is used until it is refused.
type TokenFile struct {
	path string

	mu      sync.Mutex
	token   *Token
	stamp   string
	checked time.Time
	reread  bool
}

// NewTokenFile returns a provider for the token in path.
func NewTokenFile(path string) *TokenFile { return &TokenFile{path: path, reread: true} }

// Path is the file this provider reads.
func (f *TokenFile) Path() string { return f.path }

// String never shows the token.
func (f *TokenFile) String() string { return "TokenFile(" + f.path + ")" }

// GoString never shows the token.
func (f *TokenFile) GoString() string { return f.String() }

// Token returns the token in the file.
func (f *TokenFile) Token(context.Context) (Token, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	now := time.Now()
	if !f.reread && f.token != nil && now.Sub(f.checked) < fileCheck {
		return *f.token, nil
	}
	f.checked = now
	err := f.read()
	if err != nil && (f.token == nil || f.reread) {
		f.token = nil
		return Token{}, &AuthError{Message: "cannot read the token file " + f.path + ": " + err.Error(), Err: err}
	}
	f.reread = false
	if f.token == nil {
		return Token{}, &AuthError{Message: "cannot read the token file " + f.path}
	}
	return *f.token, nil
}

func (f *TokenFile) read() error {
	info, err := os.Stat(f.path)
	if err != nil {
		return errors.New(describeFileError(err))
	}
	stamp := fmt.Sprintf("%d:%d", info.ModTime().UnixNano(), info.Size())
	if !f.reread && stamp == f.stamp && f.token != nil {
		return nil
	}
	data, err := os.ReadFile(f.path)
	if err != nil {
		return errors.New(describeFileError(err))
	}
	access := strings.TrimSpace(string(data))
	if access == "" {
		return errors.New("the file is empty")
	}
	f.token = &Token{Access: access, ExpiresAt: jwtExpiry(access)}
	f.stamp = stamp
	return nil
}

// Invalidate marks the token refused: the file is read again before the next call.
func (f *TokenFile) Invalidate() {
	f.mu.Lock()
	f.reread = true
	f.mu.Unlock()
}

// jwtExpiry is a JWT's exp claim, read and not verified; the zero time for anything
// else.
func jwtExpiry(token string) time.Time {
	parts := strings.Split(token, ".")
	if len(parts) != 3 {
		return time.Time{}
	}
	payload, err := base64.RawURLEncoding.DecodeString(strings.TrimRight(parts[1], "="))
	if err != nil {
		return time.Time{}
	}
	var claims struct {
		Exp *float64 `json:"exp"`
	}
	if json.Unmarshal(payload, &claims) != nil || claims.Exp == nil || *claims.Exp <= 0 {
		return time.Time{}
	}
	return time.Unix(int64(*claims.Exp), 0)
}

// CliToken is the developer's iohr login (docs/config.md section 5.4): it runs iohr auth
// token --profile <name> --format json without a shell, with standard input closed, a
// 10 s limit and the inherited environment, and caches the token it prints by
// CachedToken's rules.
type CliToken struct {
	profile string
	program string
	cache   *CachedToken
}

// NewCliToken returns a provider for one iohr profile; program empty runs iohr from
// PATH.
func NewCliToken(profile, program string) *CliToken {
	if program == "" {
		program = defaultCLI
	}
	c := &CliToken{profile: profile, program: program}
	c.cache = NewCachedToken(tokenFunc(c.run))
	return c
}

// String never shows the token.
func (c *CliToken) String() string { return "CliToken(" + c.profile + ")" }

// GoString never shows the token.
func (c *CliToken) GoString() string { return c.String() }

// Token returns a cached token, or a fresh one from iohr auth token.
func (c *CliToken) Token(ctx context.Context) (Token, error) { return c.cache.Token(ctx) }

// Invalidate drops the cached token, so the next call runs iohr again.
func (c *CliToken) Invalidate() { c.cache.Invalidate() }

func (c *CliToken) run(ctx context.Context) (Token, error) {
	rctx, cancel := context.WithTimeout(ctx, cliTimeout)
	defer cancel()
	cmd := exec.CommandContext(rctx, c.program, "auth", "token", "--profile", c.profile, "--format", "json") //nolint:gosec // the configured iohr, with fixed arguments and no shell
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &limited{w: &stdout, n: 1 << 20}
	cmd.Stderr = &limited{w: &stderr, n: 64 << 10}
	cmd.WaitDelay = time.Second
	err := cmd.Run()
	if rctx.Err() != nil && ctx.Err() == nil {
		return Token{}, &AuthError{Message: fmt.Sprintf("the iohr login: %s did not answer within %d s", c.program, int(cliTimeout/time.Second))}
	}
	var exit *exec.ExitError
	switch {
	case errors.As(err, &exit):
		line, _, _ := strings.Cut(stderr.String(), "\n")
		line = strings.TrimSpace(line)
		if utf8.RuneCountInString(line) > 200 {
			line = string([]rune(line)[:200])
		}
		if line == "" {
			line = "no message"
		}
		return Token{}, &AuthError{
			Message:    fmt.Sprintf("the iohr login for profile %s failed (exit %d): %s", c.profile, exit.ExitCode(), line),
			OAuthError: fmt.Sprintf("exit_%d", exit.ExitCode()),
		}
	case err != nil:
		return Token{}, &AuthError{Message: "the iohr login: cannot run " + c.program, Err: err}
	}
	// Standard output holds the token: it is parsed, never quoted in an error.
	var answer struct {
		AccessToken string  `json:"access_token"`
		ExpiresAt   *string `json:"expires_at"`
	}
	if json.Unmarshal(stdout.Bytes(), &answer) != nil || answer.AccessToken == "" {
		return Token{}, &AuthError{Message: "the iohr login: iohr auth token printed no token"}
	}
	t := Token{Access: answer.AccessToken}
	if answer.ExpiresAt != nil {
		if at, err := time.Parse(time.RFC3339, *answer.ExpiresAt); err == nil {
			t.ExpiresAt = at
		}
	}
	return t, nil
}

// limited keeps at most n bytes of what is written to it.
type limited struct {
	w interface{ Write([]byte) (int, error) }
	n int
}

func (l *limited) Write(p []byte) (int, error) {
	if l.n > 0 {
		k := min(len(p), l.n)
		_, _ = l.w.Write(p[:k])
		l.n -= k
	}
	return len(p), nil
}

// ChainedCredential tries providers in order and keeps the first that hands out a
// token. When none does, the AuthError lists why each failed.
type ChainedCredential struct {
	providers []TokenProvider

	mu     sync.Mutex
	chosen TokenProvider
}

// NewChainedCredential returns a chain of providers, tried in this order.
func NewChainedCredential(providers ...TokenProvider) *ChainedCredential {
	return &ChainedCredential{providers: append([]TokenProvider(nil), providers...)}
}

// String never shows a token.
func (c *ChainedCredential) String() string {
	return fmt.Sprintf("ChainedCredential(%d)", len(c.providers))
}

// GoString never shows a token.
func (c *ChainedCredential) GoString() string { return c.String() }

// Token returns a token from the provider chosen, choosing it on the first call.
func (c *ChainedCredential) Token(ctx context.Context) (Token, error) {
	c.mu.Lock()
	chosen := c.chosen
	c.mu.Unlock()
	if chosen != nil {
		return chosen.Token(ctx)
	}
	var failures []string
	for _, p := range c.providers {
		t, err := p.Token(ctx)
		if err == nil {
			c.mu.Lock()
			c.chosen = p
			c.mu.Unlock()
			return t, nil
		}
		failures = append(failures, fmt.Sprintf("%v: %v", p, err))
	}
	return Token{}, &AuthError{Message: "no credential in the chain gave a token; tried:\n  " + strings.Join(failures, "\n  ")}
}

// Invalidate tells the provider chosen.
func (c *ChainedCredential) Invalidate() {
	c.mu.Lock()
	chosen := c.chosen
	c.mu.Unlock()
	if inv, ok := chosen.(Invalidator); ok {
		inv.Invalidate()
	}
}

// DefaultCredential is the credential chain of docs/config.md section 5.1 as a
// TokenProvider: code, the environment, the reserved workload slot, the config file and
// the iohr login, resolved once, as Load does. Load uses the same chain; this type is
// for a program that wants the chain's token without a client.
type DefaultCredential struct {
	provider TokenProvider
	config   *ResolvedConfig
}

// NewDefaultCredential resolves the chain with opts (the options Load takes); a
// ConfigError lists every source tried when none has a credential.
func NewDefaultCredential(ctx context.Context, opts ...Option) (*DefaultCredential, error) {
	c, err := Load(ctx, opts...)
	if err != nil {
		return nil, err
	}
	return &DefaultCredential{provider: c.provider, config: c.config}, nil
}

// Config says which source was chosen and what was tried.
func (d *DefaultCredential) Config() *ResolvedConfig { return d.config }

// Token returns a token from the source chosen.
func (d *DefaultCredential) Token(ctx context.Context) (Token, error) { return d.provider.Token(ctx) }

// Invalidate tells the source chosen that its token was refused.
func (d *DefaultCredential) Invalidate() {
	if inv, ok := d.provider.(Invalidator); ok {
		inv.Invalidate()
	}
}

// String never shows a token.
func (d *DefaultCredential) String() string {
	src := "none"
	if cr := d.config.d.Credential; cr != nil {
		src = cr.Source
	}
	return "DefaultCredential(" + src + ")"
}

// GoString never shows a token.
func (d *DefaultCredential) GoString() string { return d.String() }
