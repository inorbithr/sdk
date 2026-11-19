package inorbit

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/inorbithr/sdk/go/codegen"
)

func TestAnOperationNeverPrintsItsValues(t *testing.T) {
	op := Operation{
		Name: "keys.create", Method: http.MethodPost, Path: "/v1/keys", Template: "/v1/keys",
		Query:  url.Values{"filter": {"value-SECRET"}},
		Body:   map[string]string{"secret": "body-SECRET"},
		Fields: map[string]any{"key_id": "field-SECRET"},
	}
	for _, verb := range []string{"%v", "%+v", "%#v", "%s", "%d"} {
		if s := fmt.Sprintf(verb, op); strings.Contains(s, "SECRET") || !strings.Contains(s, "filter") {
			t.Fatalf("%s: %s", verb, s)
		}
	}
	var buf bytes.Buffer
	slog.New(slog.NewJSONHandler(&buf, nil)).Info("call", "op", op)
	if strings.Contains(buf.String(), "SECRET") {
		t.Fatalf("slog: %s", buf.String())
	}
}

func TestCredentialsNeverPrintTheirSecret(t *testing.T) {
	cfg := ClientCredentialsConfig{KeyID: "ak_test", KeySecret: "s3cr3t", Scopes: []string{"identity:read"}}
	tok := Token{Access: "tok-SECRET", ExpiresAt: time.Unix(1_800_000_000, 0).UTC()}
	values := map[string]any{
		"Token":                    tok,
		"*Token":                   &tok,
		"ClientCredentialsConfig":  cfg,
		"*ClientCredentialsConfig": &cfg,
		"ClientCredentials":        NewClientCredentialsWith(cfg),
		"StaticToken":              NewStaticToken("tok-SECRET"),
		"CachedToken":              NewCachedToken(NewStaticToken("tok-SECRET")),
		"ChainedCredential":        NewChainedCredential(NewStaticToken("tok-SECRET")),
		"TokenFile":                NewTokenFile("/run/tok"),
		"CliToken":                 NewCliToken("dev", ""),
	}
	check := func(t *testing.T, how, s string) {
		t.Helper()
		if strings.Contains(s, "SECRET") || strings.Contains(s, "s3cr3t") {
			t.Fatalf("%s printed a secret: %s", how, s)
		}
	}
	for name, v := range values {
		t.Run(name, func(t *testing.T) {
			// %p is left out: on a value that is not a pointer fmt prints its bad-verb form
			// without asking the type, and a Token is a value.
			for _, verb := range []string{"%v", "%+v", "%#v", "%s", "%q", "%d", "%x", "%X", "%T %v", "%10.3v"} {
				check(t, verb, fmt.Sprintf(verb, v))
			}
			check(t, "Sprint", fmt.Sprint(v))
			check(t, "Sprintln", fmt.Sprintln(v))
			// Inside a struct and a slice, as a caller's own log line would hold it.
			check(t, "nested", fmt.Sprintf("%+v %v", struct{ V any }{v}, []any{v}))
			js, err := json.Marshal(v)
			if err != nil {
				t.Fatal(err)
			}
			check(t, "json", string(js))
			nested, _ := json.Marshal(map[string]any{"v": v})
			check(t, "nested json", string(nested))
			var jsonLog, textLog bytes.Buffer
			slog.New(slog.NewJSONHandler(&jsonLog, nil)).Info("x", "v", v, slog.Group("g", "v", v))
			slog.New(slog.NewTextHandler(&textLog, nil)).Info("x", "v", v)
			check(t, "slog json", jsonLog.String())
			check(t, "slog text", textLog.String())
		})
	}
	// What a Token and the config show is the marker, and the fields that are not secret.
	for _, s := range []string{fmt.Sprintf("%+v", tok), fmt.Sprintf("%d", cfg)} {
		if !strings.Contains(s, redacted) {
			t.Fatalf("no %s marker: %s", redacted, s)
		}
	}
	// encoding/json writes the marker as \u003credacted\u003e; read back, it is the marker.
	for v, want := range map[string]map[string]any{
		"token":  {"access": redacted, "expires_at": "2027-01-15T08:00:00Z"},
		"config": {"key_id": "ak_test", "key_secret": redacted, "scopes": []any{"identity:read"}},
	} {
		js, _ := json.Marshal(map[string]any{"token": tok, "config": cfg}[v])
		var got map[string]any
		if err := json.Unmarshal(js, &got); err != nil || fmt.Sprint(got) != fmt.Sprint(want) {
			t.Fatalf("%s json: %s", v, js)
		}
	}
}

func TestInt64TravelsAsADecimalString(t *testing.T) {
	var v struct {
		N Int64 `json:"n"`
		M Int64 `json:"m"`
	}
	if err := json.Unmarshal([]byte(`{"n":"9007199254740993","m":42}`), &v); err != nil {
		t.Fatal(err)
	}
	if v.N != 9007199254740993 || v.M != 42 {
		t.Fatalf("read %d %d", v.N, v.M)
	}
	out, _ := json.Marshal(v)
	if string(out) != `{"n":"9007199254740993","m":"42"}` {
		t.Fatalf("wrote %s", out)
	}
	if err := json.Unmarshal([]byte(`{"n":"x"}`), &v); err == nil {
		t.Fatal("a non-number was read")
	}
}

func TestFromEnvNamesWhatToSet(t *testing.T) {
	t.Setenv("INORBIT_TOKEN", "bare")
	_, err := FromEnv("ACME_CI")
	var cerr *ConfigError
	if !errors.As(err, &cerr) || !strings.Contains(cerr.Message, "INORBIT_ACME_CI_TOKEN") {
		t.Fatalf("a named profile must not fall back to INORBIT_TOKEN: %v", err)
	}
	t.Setenv("INORBIT_ACME_CI_KEY_ID", "ak_1")
	t.Setenv("INORBIT_ACME_CI_KEY_SECRET", "s")
	if _, err := FromEnv("ACME_CI"); err == nil || !strings.Contains(err.Error(), "INORBIT_ACME_CI_SCOPES") {
		t.Fatalf("want a scopes error naming the variable, got %v", err)
	}
	if _, err := FromEnv(""); err != nil {
		t.Fatalf("the bare names: %v", err)
	}
}

func TestURLsMustBeHTTPSOrLoopback(t *testing.T) {
	for _, bad := range []string{"http://api.example.com", "https://u:p@api.inorbit.hr", "https://api.inorbit.hr/v1", "ftp://x", "http://127.example.com", "http://127.0.0.1.example.com", "http://localhost.example.com"} {
		if _, err := NewClient(WithToken("t"), WithBaseURL(bad)); err == nil {
			t.Fatalf("accepted %s", bad)
		}
	}
	for _, ok := range []string{"http://127.0.0.1:8080", "http://127.1.2.3", "http://[::1]:8080", "http://localhost:3000"} {
		if _, err := NewClient(WithToken("t"), WithBaseURL(ok)); err != nil {
			t.Fatalf("loopback %s: %v", ok, err)
		}
	}
	if _, err := NewClient(); err == nil {
		t.Fatal("a client without credentials")
	}
	// The first check NewClient makes agrees with the settings check: a name that only
	// starts like a loopback address is a remote host.
	for _, bad := range []string{"http://127.example.com", "http://127.0.0.1.example.com"} {
		if _, err := checkURL("the base URL", bad, true); err == nil {
			t.Fatalf("checkURL accepted %s", bad)
		}
	}
}

func TestAPathParameterIsOneSegment(t *testing.T) {
	if got := codegen.PathSegment("a/b c?~é"); got != "a%2Fb%20c%3F~%C3%A9" {
		t.Fatalf("got %s", got)
	}
}

func TestAPlainTextGatewayErrorIsReadByStatus(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusForbidden)
		_, _ = fmt.Fprint(w, "RBAC: access denied")
	}))
	defer srv.Close()
	c, _ := NewClient(WithBaseURL(srv.URL), WithToken("t"))
	_, err := c.Send(context.Background(), Operation{Method: "GET", Path: "/v1/me"})
	var aerr *APIError
	if !errors.As(err, &aerr) || aerr.Code != CodeForbidden || aerr.Status != 403 {
		t.Fatalf("got %v", err)
	}
}
