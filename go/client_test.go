package inorbit

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/inorbithr/sdk/go/codegen"
)

func TestCredentialsNeverPrintTheirSecret(t *testing.T) {
	cc := NewClientCredentials("ak_test", "s3cr3t", []string{"identity:read"}, "", nil)
	st := NewStaticToken("tok-SECRET")
	tok := Token{Access: "tok-SECRET"}
	for _, s := range []string{
		fmt.Sprint(cc), fmt.Sprintf("%#v", cc), fmt.Sprintf("%v", st), fmt.Sprintf("%#v", st),
		fmt.Sprint(tok), fmt.Sprintf("%#v", tok),
	} {
		if strings.Contains(s, "SECRET") || strings.Contains(s, "s3cr3t") {
			t.Fatalf("a secret printed: %s", s)
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
	for _, bad := range []string{"http://api.example.com", "https://u:p@api.inorbit.hr", "https://api.inorbit.hr/v1", "ftp://x"} {
		if _, err := NewClient(WithToken("t"), WithBaseURL(bad)); err == nil {
			t.Fatalf("accepted %s", bad)
		}
	}
	if _, err := NewClient(WithToken("t"), WithBaseURL("http://127.0.0.1:8080")); err != nil {
		t.Fatalf("loopback: %v", err)
	}
	if _, err := NewClient(); err == nil {
		t.Fatal("a client without credentials")
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
