package inorbit

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"net/http"
	"net/url"
	"os"
	"strings"
	"time"
)

// Audience is the audience every token for the API is asked for.
const Audience = "iohr-api"

// DefaultTokenURL is where an API key is exchanged for a token.
const DefaultTokenURL = "https://auth.inorbit.hr/oauth2/token" //nolint:gosec // a URL, not a credential

const (
	tokenRetries = 2
	tokenTimeout = 30 * time.Second
	redacted     = "<redacted>"
)

// Token is a bearer token and, when known, when it stops working.
type Token struct {
	// Access is the token. Never log it.
	Access string

	// ExpiresAt is when it expires; the zero time when the provider does not know.
	ExpiresAt time.Time
}

// String never shows the token.
func (Token) String() string { return "Token(" + redacted + ")" }

// GoString never shows the token.
func (t Token) GoString() string { return t.String() }

// TokenProvider hands out a valid token for each attempt.
type TokenProvider interface {
	// Token returns a token for the next attempt.
	Token(ctx context.Context) (Token, error)
}

// Invalidator is a TokenProvider that drops a cached token when the API refused it
// (HTTP 401), so the next attempt gets a fresh one.
type Invalidator interface {
	// Invalidate drops any cached token.
	Invalidate()
}

// StaticToken is a token you already hold, such as an API token from the console.
type StaticToken struct {
	token string
}

// NewStaticToken returns a provider that always hands out token.
func NewStaticToken(token string) *StaticToken { return &StaticToken{token: token} }

// Token returns the token.
func (s *StaticToken) Token(context.Context) (Token, error) {
	return Token{Access: s.token}, nil
}

// String never shows the token.
func (*StaticToken) String() string { return "StaticToken(" + redacted + ")" }

// GoString never shows the token.
func (s *StaticToken) GoString() string { return s.String() }

// ClientCredentials exchanges an API key for 15-minute tokens (OAuth client
// credentials), caches the token by CachedToken's rules (refreshed when less than a fifth
// of its life is left or after a 401, one exchange however many calls wait for it, the
// valid token kept when a refresh fails), and reads a secret file before every exchange,
// so a rotated secret is used at the next one.
type ClientCredentials struct {
	keyID      string
	secret     string
	secretFile string
	scopes     []string
	tokenURL   string
	http       *http.Client
	userAgent  string
	cache      *CachedToken
}

// ClientCredentialsConfig configures NewClientCredentialsWith.
type ClientCredentialsConfig struct {
	// KeyID is the key's id (ak_...).
	KeyID string

	// KeySecret is the key's secret; leave it empty to read KeySecretFile instead.
	KeySecret string

	// KeySecretFile is a file holding the secret, read before every token exchange.
	KeySecretFile string

	// Scopes is what to ask for, a subset of the key's.
	Scopes []string

	// TokenURL is the token endpoint; empty for DefaultTokenURL.
	TokenURL string

	// HTTPClient sends the exchange; nil for a client of its own.
	HTTPClient *http.Client

	// UserAgent is sent with the exchange; empty for the SDK's.
	UserAgent string
}

// NewClientCredentials returns a provider for one key. scopes is what to ask for, a
// subset of the key's; tokenURL empty means DefaultTokenURL; httpClient nil means a
// client of its own.
func NewClientCredentials(keyID, keySecret string, scopes []string, tokenURL string, httpClient *http.Client) *ClientCredentials {
	return NewClientCredentialsWith(ClientCredentialsConfig{
		KeyID: keyID, KeySecret: keySecret, Scopes: scopes, TokenURL: tokenURL, HTTPClient: httpClient,
	})
}

// NewClientCredentialsWith returns a provider for the key cfg describes, its secret given
// or read from a file before every exchange.
func NewClientCredentialsWith(cfg ClientCredentialsConfig) *ClientCredentials {
	if cfg.TokenURL == "" {
		cfg.TokenURL = DefaultTokenURL
	}
	if cfg.HTTPClient == nil {
		cfg.HTTPClient = &http.Client{CheckRedirect: noRedirects}
	}
	if cfg.UserAgent == "" {
		cfg.UserAgent = userAgent("")
	}
	c := &ClientCredentials{
		keyID: cfg.KeyID, secret: cfg.KeySecret, secretFile: cfg.KeySecretFile,
		scopes: append([]string(nil), cfg.Scopes...), tokenURL: cfg.TokenURL, http: cfg.HTTPClient,
		userAgent: cfg.UserAgent,
	}
	c.cache = NewCachedToken(tokenFunc(c.exchange))
	return c
}

// KeyID is the key this provider exchanges.
func (c *ClientCredentials) KeyID() string { return c.keyID }

// String never shows the secret.
func (c *ClientCredentials) String() string {
	return fmt.Sprintf("ClientCredentials(%s, secret: %s)", c.keyID, redacted)
}

// GoString never shows the secret.
func (c *ClientCredentials) GoString() string { return c.String() }

// Token returns a cached token while four fifths of its life are left, else a fresh
// one; concurrent callers share one exchange.
func (c *ClientCredentials) Token(ctx context.Context) (Token, error) {
	return c.cache.Token(ctx)
}

// Invalidate drops the cached token, so the next call exchanges again.
func (c *ClientCredentials) Invalidate() { c.cache.Invalidate() }

// tokenFunc is a function that is a TokenProvider.
type tokenFunc func(ctx context.Context) (Token, error)

func (f tokenFunc) Token(ctx context.Context) (Token, error) { return f(ctx) }

func (c *ClientCredentials) readSecret() (string, error) {
	if c.secretFile == "" {
		return c.secret, nil
	}
	data, err := os.ReadFile(c.secretFile)
	if err != nil {
		return "", &AuthError{Message: "cannot read the key secret file " + c.secretFile + ": " + describeFileError(err), Err: err}
	}
	s := strings.TrimSpace(string(data))
	if s == "" {
		return "", &AuthError{Message: "the key secret file " + c.secretFile + " is empty"}
	}
	return s, nil
}

func (c *ClientCredentials) exchange(ctx context.Context) (Token, error) {
	secret, err := c.readSecret()
	if err != nil {
		return Token{}, err
	}
	form := url.Values{
		"grant_type": {"client_credentials"},
		"audience":   {Audience},
		"scope":      {strings.Join(c.scopes, " ")},
	}.Encode()
	for attempt := 0; ; attempt++ {
		actx, cancel := context.WithTimeout(ctx, tokenTimeout)
		req, err := http.NewRequestWithContext(actx, http.MethodPost, c.tokenURL, strings.NewReader(form))
		if err != nil {
			cancel()
			return Token{}, &AuthError{Message: "the token endpoint is not usable: " + err.Error(), Err: err}
		}
		req.SetBasicAuth(url.QueryEscape(c.keyID), url.QueryEscape(secret))
		req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
		req.Header.Set("User-Agent", c.userAgent)
		resp, err := c.http.Do(req)
		if err != nil {
			cancel()
			if attempt < tokenRetries {
				if serr := sleep(ctx, backoff(attempt)); serr != nil {
					return Token{}, &AuthError{Message: "the token endpoint: " + serr.Error(), Err: serr}
				}
				continue
			}
			return Token{}, &AuthError{Message: "the token endpoint: " + describe(err), Err: err}
		}
		body, rerr := io.ReadAll(io.LimitReader(resp.Body, maxBody))
		_ = resp.Body.Close()
		cancel()
		if retryableStatus(resp.StatusCode) && attempt < tokenRetries {
			wait, ok := retryAfter(resp.Header, time.Now())
			if !ok {
				wait = backoff(attempt)
			}
			if serr := sleep(ctx, min(wait, retryAfterCap)); serr != nil {
				return Token{}, &AuthError{Message: "the token endpoint: " + serr.Error(), Err: serr}
			}
			continue
		}
		if rerr != nil {
			return Token{}, &AuthError{Message: "the token endpoint: " + describe(rerr), Err: rerr}
		}
		if resp.StatusCode < 200 || resp.StatusCode >= 300 {
			var refusal struct {
				Error       string `json:"error"`
				Description string `json:"error_description"`
			}
			_ = json.Unmarshal(body, &refusal)
			code := refusal.Error
			if code == "" {
				code = fmt.Sprintf("HTTP %d", resp.StatusCode)
			}
			desc := ""
			if refusal.Description != "" {
				desc = " (" + refusal.Description + ")"
			}
			return Token{}, &AuthError{
				Message:    fmt.Sprintf("the token exchange for key %s failed: %s%s", c.keyID, code, desc),
				OAuthError: code,
			}
		}
		var answer struct {
			AccessToken string  `json:"access_token"`
			ExpiresIn   float64 `json:"expires_in"`
		}
		if err := json.Unmarshal(body, &answer); err != nil || answer.AccessToken == "" {
			return Token{}, &AuthError{Message: "the token endpoint: the token answer could not be read"}
		}
		lifetime := 900 * time.Second
		if answer.ExpiresIn > 0 {
			lifetime = time.Duration(answer.ExpiresIn * float64(time.Second))
		}
		return Token{Access: answer.AccessToken, ExpiresAt: time.Now().Add(lifetime)}, nil
	}
}

// describe is a transport failure in words, without the request or its URL.
func describe(err error) string {
	var uerr *url.Error
	if errors.As(err, &uerr) {
		return uerr.Err.Error()
	}
	return err.Error()
}

func noRedirects(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }

// describeFileError is a file failure in words, without repeating the path.
func describeFileError(err error) string {
	switch {
	case errors.Is(err, fs.ErrNotExist):
		return "it does not exist"
	case errors.Is(err, fs.ErrPermission):
		return "permission denied"
	}
	var perr *fs.PathError
	if errors.As(err, &perr) {
		return perr.Err.Error()
	}
	return err.Error()
}
