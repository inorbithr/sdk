package inorbit

import (
	"bytes"
	"context"
	"crypto/aes"
	"crypto/cipher"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/pbkdf2"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/asn1"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync/atomic"
	"testing"
	"testing/synctest"
	"time"
)

func memClient(t *testing.T, rt RoundTripperFunc, opts ...Option) *Client {
	t.Helper()
	all := append([]Option{WithToken("tok"), WithBaseURL("https://api.test"), WithTransport(rt)}, opts...)
	c, err := NewClient(all...)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

func TestThePipelineIsEditedByName(t *testing.T) {
	probe := func(name string) Middleware { return Middleware{Name: name, Wrap: passThrough} }
	c := memClient(t, func(*http.Request) (*http.Response, error) { return answer(200, `{}`), nil },
		WithPipeline(func(p *Pipeline) {
			p.AddPerCall(probe("a")).AddPerRetry(probe("b")).InsertBefore("auth", probe("c")).
				InsertAfter("user_agent", probe("d")).Replace("rate_limit", probe("x")).Remove("logging")
		}))
	want := "request_id user_agent d idempotency_key call_tracing deadline a retry c auth rate_limit attempt_tracing hooks b timeout"
	if got := strings.Join(c.Config().Describe().Pipeline, " "); got != want {
		t.Fatalf("pipeline\n got %s\nwant %s", got, want)
	}
	for _, edit := range []func(*Pipeline){
		func(p *Pipeline) { p.Remove("retry") },
		func(p *Pipeline) { p.Remove("auth") },
		func(p *Pipeline) { p.Remove("timeout") },
		func(p *Pipeline) { p.AddPerCall(probe("hooks")) },
		func(p *Pipeline) { p.InsertBefore("nope", probe("a")) },
	} {
		_, err := NewClient(WithToken("t"), WithPipeline(edit))
		var cerr *ConfigError
		if !errors.As(err, &cerr) {
			t.Fatalf("want a ConfigError, got %v", err)
		}
	}
}

func TestAMiddlewareSeesCallInfoAndCanAnswer(t *testing.T) {
	var infos []CallInfo
	c := memClient(t, func(*http.Request) (*http.Response, error) {
		t.Fatal("the transport was called")
		return nil, nil
	}, WithPipeline(func(p *Pipeline) {
		p.AddPerRetry(Middleware{Name: "answer", Wrap: func(http.RoundTripper) http.RoundTripper {
			return RoundTripperFunc(func(r *http.Request) (*http.Response, error) {
				info, _ := CallInfoFrom(r.Context())
				infos = append(infos, info)
				return answer(200, `{"ok":true}`), nil
			})
		}})
	}))
	raw, err := c.Send(t.Context(), Operation{Name: "me", Method: "GET", Path: "/v1/me", Template: "/v1/me"})
	if err != nil || string(raw.Body) != `{"ok":true}` {
		t.Fatalf("%v %v", raw, err)
	}
	i := infos[0]
	if i.Operation != "me" || i.Template != "/v1/me" || i.Attempt != 1 || i.Stage != StagePerRetry ||
		!strings.HasPrefix(i.RequestID, "iohr-") || i.Deadline.IsZero() || !i.Idempotent {
		t.Fatalf("info %+v", i)
	}
}

func TestAnIdempotencyKeyOnAnOperationWithoutOneIsAConfigError(t *testing.T) {
	c := memClient(t, func(*http.Request) (*http.Response, error) { return answer(200, `{}`), nil })
	ctx := WithCallOptions(t.Context(), CallOptions{IdempotencyKey: "k"})
	_, err := c.Send(ctx, Operation{Name: "events.update_endpoint", Method: "PATCH", Path: "/v1/x"})
	var cerr *ConfigError
	if !errors.As(err, &cerr) {
		t.Fatalf("want a ConfigError, got %v", err)
	}
}

func TestAKeyedWriteKeepsItsKeyAcrossAttemptsAndOnTheError(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var keys []string
		c := memClient(t, func(r *http.Request) (*http.Response, error) {
			keys = append(keys, r.Header.Get("Idempotency-Key"))
			return nil, errors.New("reset")
		}, WithMaxRetries(1))
		_, err := c.Send(t.Context(), Operation{Name: "create", Method: "POST", Path: "/v1/x", Body: map[string]any{}, IdempotencyKey: true})
		var ce *ConnectionError
		if !errors.As(err, &ce) || len(keys) != 2 || keys[0] == "" || keys[0] != keys[1] || ce.IdempotencyKey != keys[0] {
			t.Fatalf("err %v, keys %v", err, keys)
		}
	})
}

func TestTheRetryBudgetRunsDryAndRefills(t *testing.T) {
	b := newRetryBudget(20)
	first, second, third := b.take(10), b.take(10), b.take(5)
	if !first || !second || third {
		t.Fatal("a bucket of 20 pays two retries of 10 and no more")
	}
	b.give(1)
	b.give(100)
	if b.tokens != 20 {
		t.Fatalf("tokens %d, want the capacity", b.tokens)
	}
}

func TestARefusedStaticTokenFromLoadIsAnAuthError(t *testing.T) {
	c, err := Load(t.Context(), WithTransport(RoundTripperFunc(func(*http.Request) (*http.Response, error) {
		return answer(401, ""), nil
	})), WithLoadOptions(LoadOptions{Env: map[string]string{"INORBIT_TOKEN": "t", "INORBIT_CONFIG_FILE": "off"}, NoHome: true}))
	if err != nil {
		t.Fatal(err)
	}
	_, err = c.Send(t.Context(), Operation{Name: "me", Method: "GET", Path: "/v1/me"})
	var aerr *AuthError
	if !errors.As(err, &aerr) || !strings.Contains(aerr.Message, "refused the token") {
		t.Fatalf("want an AuthError, got %v", err)
	}
}

func TestDescribeIsTheDocumentedJSON(t *testing.T) {
	cfg, err := LoadConfig(t.Context(), WithTimeout(5*time.Second), WithLoadOptions(LoadOptions{
		Env: map[string]string{"INORBIT_KEY_ID": "ak_1", "INORBIT_KEY_SECRET": "s3cr3t", "INORBIT_SCOPES": "a b", "INORBIT_CONFIG_FILE": "off"}, NoHome: true,
	}))
	if err != nil {
		t.Fatal(err)
	}
	b, _ := json.Marshal(cfg)
	var doc map[string]any
	_ = json.Unmarshal(b, &doc)
	for _, k := range []string{"profile", "config_file", "settings", "credential", "pipeline", "ignored"} {
		if _, ok := doc[k]; !ok {
			t.Fatalf("no %s in %s", k, b)
		}
	}
	if bytes.Contains(b, []byte("s3cr3t")) || !bytes.Contains(b, []byte(`"timeout":{"value":"5s","source":"code"}`)) {
		t.Fatalf("describe: %s", b)
	}
}

func TestTheUserAgentUsesTheSharedVocabulary(t *testing.T) {
	ua := userAgent("app/1")
	if !strings.HasPrefix(ua, "inorbithr-sdk-go/"+SDKVersion+" go/") || !strings.HasSuffix(ua, " app/1") {
		t.Fatalf("user agent %q", ua)
	}
	if uaOS("darwin") != "macos" || uaArch("amd64") != "x86_64" || uaArch("arm64") != "aarch64" || uaOS("plan9") != "other" {
		t.Fatal("os and arch are not normalised")
	}
}

type countingSource struct {
	calls    atomic.Int32
	fail     atomic.Bool
	lifetime time.Duration
}

func (s *countingSource) Token(context.Context) (Token, error) {
	n := s.calls.Add(1)
	time.Sleep(time.Second)
	if s.fail.Load() {
		return Token{}, errors.New("down")
	}
	return Token{Access: fmt.Sprintf("t%d", n), ExpiresAt: time.Now().Add(s.lifetime)}, nil
}

func TestCachedTokenRefreshesAheadOnceAndKeepsAValidToken(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		src := &countingSource{lifetime: 100 * time.Second}
		c := NewCachedToken(src)
		done := make(chan string, 5)
		for range 5 {
			go func() { tok, _ := c.Token(t.Context()); done <- tok.Access }()
		}
		for range 5 {
			if got := <-done; got != "t1" {
				t.Fatalf("token %s, want t1 from one fetch", got)
			}
		}
		time.Sleep(79 * time.Second)
		if tok, _ := c.Token(t.Context()); tok.Access != "t1" || src.calls.Load() != 1 {
			t.Fatalf("refreshed before four fifths of the life: %s", tok.Access)
		}
		time.Sleep(2 * time.Second)
		src.fail.Store(true)
		failures := 0
		c.onRefreshFailed = func(error) { failures++ }
		if tok, err := c.Token(t.Context()); err != nil || tok.Access != "t1" || failures != 1 {
			t.Fatalf("a failed refresh with a valid token: %v %v %d", tok.Access, err, failures)
		}
		calls := src.calls.Load()
		if _, _ = c.Token(t.Context()); src.calls.Load() != calls {
			t.Fatal("refreshed again within 5 s of a failure")
		}
		time.Sleep(30 * time.Second)
		if _, err := c.Token(t.Context()); err == nil {
			t.Fatal("an expired token and a failing source must fail")
		}
	})
}

func TestATokenFileIsReadAgainWhenRefusedAndKeptWhenGone(t *testing.T) {
	path := filepath.Join(t.TempDir(), "token")
	write := func(s string) {
		if err := os.WriteFile(path, []byte(s+"\n"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	write("tok-1")
	f := NewTokenFile(path)
	if tok, err := f.Token(t.Context()); err != nil || tok.Access != "tok-1" {
		t.Fatalf("%v %v", tok, err)
	}
	write("tok-2")
	if tok, _ := f.Token(t.Context()); tok.Access != "tok-1" {
		t.Fatal("the file is checked at most once a minute")
	}
	f.Invalidate()
	if tok, _ := f.Token(t.Context()); tok.Access != "tok-2" {
		t.Fatal("a refused token is read again at once")
	}
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	f.Invalidate()
	if _, err := f.Token(t.Context()); err == nil || !strings.Contains(err.Error(), path) {
		t.Fatalf("a refused token whose file is gone: %v", err)
	}
	if fmt.Sprint(f) != "TokenFile("+path+")" {
		t.Fatal("String names the file, never the token")
	}
}

func TestAJWTExpiryIsRead(t *testing.T) {
	payload := `{"exp":1900000000}`
	tok := "e30." + strings.TrimRight(b64url(payload), "=") + ".sig"
	if got := jwtExpiry(tok); got.Unix() != 1900000000 {
		t.Fatalf("exp %v", got)
	}
	if !jwtExpiry("opaque").IsZero() {
		t.Fatal("an opaque token has no expiry")
	}
}

func b64url(s string) string {
	const enc = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
	var out strings.Builder
	b := []byte(s)
	for i := 0; i < len(b); i += 3 {
		var n uint32
		k := min(3, len(b)-i)
		for j := range 3 {
			n <<= 8
			if j < k {
				n |= uint32(b[i+j])
			}
		}
		for j := range k + 1 {
			out.WriteByte(enc[(n>>(18-6*j))&63])
		}
	}
	return out.String()
}

func TestAFailingIohrIsAnAuthErrorWithItsFirstLine(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("a shell script stands in for iohr")
	}
	dir := t.TempDir()
	prog := filepath.Join(dir, "iohr")
	script := "#!/bin/sh\necho 'iohr: not signed in; run iohr login' >&2\necho second >&2\nexit 3\n"
	if err := os.WriteFile(prog, []byte(script), 0o700); err != nil { //nolint:gosec // an executable stand-in
		t.Fatal(err)
	}
	_, err := NewCliToken("dev", prog).Token(t.Context())
	var aerr *AuthError
	if !errors.As(err, &aerr) || aerr.Message != "the iohr login for profile dev failed (exit 3): iohr: not signed in; run iohr login" || aerr.OAuthError != "exit_3" {
		t.Fatalf("got %v", err)
	}
}

func TestCredentialsAndCachesNeverPrintTheirToken(t *testing.T) {
	for _, v := range []any{NewCachedToken(NewStaticToken("s3cr3t")), NewCliToken("dev", ""), NewChainedCredential(NewStaticToken("s3cr3t"))} {
		if s := fmt.Sprintf("%v %#v", v, v); strings.Contains(s, "s3cr3t") {
			t.Fatalf("%s", s)
		}
	}
}

func TestAChainKeepsTheFirstProviderThatWorks(t *testing.T) {
	failing := tokenFunc(func(context.Context) (Token, error) { return Token{}, errors.New("no") })
	c := NewChainedCredential(failing, NewStaticToken("t"))
	if tok, err := c.Token(t.Context()); err != nil || tok.Access != "t" {
		t.Fatalf("%v %v", tok, err)
	}
	if _, err := NewChainedCredential(failing).Token(t.Context()); err == nil || !strings.Contains(err.Error(), "tried") {
		t.Fatalf("got %v", err)
	}
}

func encryptPKCS8(t *testing.T, der []byte, password string) []byte {
	t.Helper()
	salt, iv := make([]byte, 16), make([]byte, 16)
	_, _ = rand.Read(salt)
	_, _ = rand.Read(iv)
	key, err := pbkdf2.Key(sha256.New, password, salt, 2048, 32)
	if err != nil {
		t.Fatal(err)
	}
	pad := aes.BlockSize - len(der)%aes.BlockSize
	plain := append(append([]byte{}, der...), bytes.Repeat([]byte{byte(pad)}, pad)...)
	block, _ := aes.NewCipher(key)
	out := make([]byte, len(plain))
	cipher.NewCBCEncrypter(block, iv).CryptBlocks(out, plain)
	must := func(b []byte, err error) asn1.RawValue {
		if err != nil {
			t.Fatal(err)
		}
		return asn1.RawValue{FullBytes: b}
	}
	kdf := must(asn1.Marshal(pbkdf2Params{Salt: salt, Iterations: 2048, PRF: pkix.AlgorithmIdentifier{Algorithm: oidHMACSHA256, Parameters: asn1.NullRawValue}}))
	ivRaw := must(asn1.Marshal(iv))
	params := must(asn1.Marshal(pbes2Params{
		KeyDerivation: pkix.AlgorithmIdentifier{Algorithm: oidPBKDF2, Parameters: kdf},
		Encryption:    pkix.AlgorithmIdentifier{Algorithm: oidAES256CBC, Parameters: ivRaw},
	}))
	b, err := asn1.Marshal(encryptedPrivateKeyInfo{Algorithm: pkix.AlgorithmIdentifier{Algorithm: oidPBES2, Parameters: params}, Data: out})
	if err != nil {
		t.Fatal(err)
	}
	return b
}

func TestAnEncryptedPKCS8KeyIsDecrypted(t *testing.T) {
	k, _ := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	der, _ := x509.MarshalPKCS8PrivateKey(k)
	enc := encryptPKCS8(t, der, "pw")
	got, err := decryptPKCS8(enc, []byte("pw"))
	if err != nil || !bytes.Equal(got, der) {
		t.Fatalf("%v", err)
	}
	if _, err := decryptPKCS8(enc, []byte("wrong")); err == nil {
		t.Fatal("a wrong password must fail")
	}
}

func TestLogsGoToTheLoggerWithoutSecrets(t *testing.T) {
	var buf bytes.Buffer
	logger := slog.New(slog.NewJSONHandler(&buf, &slog.HandlerOptions{Level: slog.LevelDebug}))
	c := memClient(t, func(*http.Request) (*http.Response, error) {
		return answer(200, `{"secret":"body-marker"}`, "X-Request-Id", "srv-1"), nil
	}, WithLog(LogDebug), WithLogHeaders(true), WithLogger(logger))
	if _, err := c.Send(t.Context(), Operation{Name: "me", Method: "GET", Path: "/v1/me", Query: map[string][]string{"q": {"query-marker"}}}); err != nil {
		t.Fatal(err)
	}
	out := buf.String()
	for _, x := range []string{"body-marker", "query-marker", "Bearer"} {
		if strings.Contains(out, x) {
			t.Fatalf("%q in the log:\n%s", x, out)
		}
	}
	for _, x := range []string{`"event":"request"`, `"event":"response"`, `"event":"call"`, `"server_request_id":"srv-1"`, `"authorization":"REDACTED"`} {
		if !strings.Contains(out, x) {
			t.Fatalf("%s not in the log:\n%s", x, out)
		}
	}
}
