package inorbit

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"sync"
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
// credentials), caches the token, refreshes it when less than a fifth of its life is
// left or after a 401, and makes one exchange however many calls wait for it.
type ClientCredentials struct {
	keyID    string
	secret   string
	scopes   []string
	tokenURL string
	http     *http.Client

	mu      sync.Mutex
	cache   *cachedToken
	pending *exchange
}

type cachedToken struct {
	access   string
	issued   time.Time
	lifetime time.Duration
}

type exchange struct {
	done  chan struct{}
	token *cachedToken
	err   error
}

// NewClientCredentials returns a provider for one key. scopes is what to ask for, a
// subset of the key's; tokenURL empty means DefaultTokenURL; httpClient nil means a
// client of its own.
func NewClientCredentials(keyID, keySecret string, scopes []string, tokenURL string, httpClient *http.Client) *ClientCredentials {
	if tokenURL == "" {
		tokenURL = DefaultTokenURL
	}
	if httpClient == nil {
		httpClient = &http.Client{CheckRedirect: noRedirects}
	}
	return &ClientCredentials{
		keyID: keyID, secret: keySecret, scopes: append([]string(nil), scopes...),
		tokenURL: tokenURL, http: httpClient,
	}
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
	c.mu.Lock()
	if t := c.cache; t != nil && time.Since(t.issued) < t.lifetime*4/5 {
		c.mu.Unlock()
		return Token{Access: t.access, ExpiresAt: t.issued.Add(t.lifetime)}, nil
	}
	ex := c.pending
	if ex == nil {
		ex = &exchange{done: make(chan struct{})}
		c.pending = ex
		go func() {
			// The exchange outlives a caller that gives up: the others still wait for it.
			t, err := c.exchange(context.WithoutCancel(ctx))
			c.mu.Lock()
			ex.token, ex.err = t, err
			if err == nil {
				c.cache = t
			}
			c.pending = nil
			c.mu.Unlock()
			close(ex.done)
		}()
	}
	c.mu.Unlock()
	select {
	case <-ctx.Done():
		return Token{}, ctx.Err()
	case <-ex.done:
	}
	if ex.err != nil {
		return Token{}, ex.err
	}
	return Token{Access: ex.token.access, ExpiresAt: ex.token.issued.Add(ex.token.lifetime)}, nil
}

// Invalidate drops the cached token, so the next call exchanges again.
func (c *ClientCredentials) Invalidate() {
	c.mu.Lock()
	c.cache = nil
	c.mu.Unlock()
}

func (c *ClientCredentials) exchange(ctx context.Context) (*cachedToken, error) {
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
			return nil, &AuthError{Message: "the token endpoint is not usable: " + err.Error(), Err: err}
		}
		req.SetBasicAuth(url.QueryEscape(c.keyID), url.QueryEscape(c.secret))
		req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
		resp, err := c.http.Do(req)
		if err != nil {
			cancel()
			if attempt < tokenRetries {
				if serr := sleep(ctx, backoff(attempt)); serr != nil {
					return nil, &AuthError{Message: "the token endpoint: " + serr.Error(), Err: serr}
				}
				continue
			}
			return nil, &AuthError{Message: "the token endpoint: " + describe(err), Err: err}
		}
		body, rerr := io.ReadAll(io.LimitReader(resp.Body, maxBody))
		_ = resp.Body.Close()
		cancel()
		if retryableStatus(resp.StatusCode) && attempt < tokenRetries {
			wait, ok := retryAfter(resp.Header)
			if !ok {
				wait = backoff(attempt)
			}
			if serr := sleep(ctx, wait); serr != nil {
				return nil, &AuthError{Message: "the token endpoint: " + serr.Error(), Err: serr}
			}
			continue
		}
		if rerr != nil {
			return nil, &AuthError{Message: "the token endpoint: " + describe(rerr), Err: rerr}
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
			return nil, &AuthError{
				Message:    fmt.Sprintf("the token exchange for key %s failed: %s%s", c.keyID, code, desc),
				OAuthError: code,
			}
		}
		var answer struct {
			AccessToken string  `json:"access_token"`
			ExpiresIn   float64 `json:"expires_in"`
		}
		if err := json.Unmarshal(body, &answer); err != nil || answer.AccessToken == "" {
			return nil, &AuthError{Message: "the token endpoint: the token answer could not be read"}
		}
		lifetime := 900 * time.Second
		if answer.ExpiresIn > 0 {
			lifetime = time.Duration(answer.ExpiresIn * float64(time.Second))
		}
		return &cachedToken{access: answer.AccessToken, issued: time.Now(), lifetime: lifetime}, nil
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
