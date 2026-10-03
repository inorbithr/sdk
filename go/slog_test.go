package inorbit

import (
	"bytes"
	"log/slog"
	"net/http"
	"net/url"
	"strings"
	"testing"
	"testing/synctest"
)

func TestSlogHookLogsTheCallButNothingItCarries(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var out bytes.Buffer
		logger := slog.New(slog.NewTextHandler(&out, &slog.HandlerOptions{Level: slog.LevelDebug}))
		f := &fakeAPI{expiresIn: 900, api: func(n int, _ *http.Request) *http.Response {
			if n == 1 {
				return answer(503, "", "Retry-After", "1")
			}
			if n == 2 {
				return answer(200, `{"secret_field":"body-value-123"}`)
			}
			return answer(404, `{"code":"not_found","error":"no such endpoint"}`)
		}}
		c := f.client(t, WithHook(SlogHook(logger)))
		op := Operation{
			Name: "events.get_endpoint", Method: "GET",
			Path:  "/v1/webhooks/endpoints/" + "ep-private-id-42",
			Query: url.Values{"status": {"query-value-77"}},
		}
		if _, err := c.Send(t.Context(), op); err != nil {
			t.Fatal(err)
		}
		if _, err := c.Send(t.Context(), Operation{Method: "GET", Path: "/v1/raw/raw-private-id-9"}); err == nil {
			t.Fatal("want the 404")
		}
		logged := out.String()
		for _, want := range []string{
			"operation=events.get_endpoint", "status=503", "status=200", "attempt=2",
			"duration=0s", "level=DEBUG", "level=WARN", "operation=raw", "request_id=iohr-",
		} {
			if !strings.Contains(logged, want) {
				t.Errorf("the log lacks %q:\n%s", want, logged)
			}
		}
		for _, leak := range []string{
			"s3cr3t", "Bearer", "tok\"", "ep-private-id-42", "raw-private-id-9",
			"query-value-77", "body-value-123", "Authorization",
		} {
			if strings.Contains(logged, leak) {
				t.Errorf("the log shows %q:\n%s", leak, logged)
			}
		}
	})
}
