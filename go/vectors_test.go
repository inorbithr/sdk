package inorbit

import (
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"runtime"
	"slices"
	"strings"
	"testing"
	"time"
)

// The pure-function vectors of conformance/vectors/ (docs/config.md section 9.2):
// configuration resolution, config file paths, no_proxy, rate-limit headers and
// durations, each run against the runtime without a server. The replay binary reads the
// YAML (replay vectors DIR), so the module needs no YAML reader; without it (mise run
// conformance:server:build) the test skips, unless IOHR_TEST_REQUIRE_REPLAY is set.

type vector struct {
	Name    string         `json:"name"`
	Kind    string         `json:"kind"`
	Pending []string       `json:"pending"`
	Input   vectorInput    `json:"input"`
	Expect  map[string]any `json:"expect"`
	Checks  []vectorCheck  `json:"checks"`
}

type vectorInput struct {
	Code        map[string]any    `json:"code"`
	Env         map[string]string `json:"env"`
	ConfigFile  *string           `json:"config_file"`
	Home        bool              `json:"home"`
	Files       map[string]string `json:"files"`
	OS          string            `json:"os"`
	ProfileType string            `json:"profile_type"`
	CLI         string            `json:"cli"`
}

type vectorCheck struct {
	Summary string            `json:"summary"`
	OS      string            `json:"os"`
	Home    *string           `json:"home"`
	Env     map[string]string `json:"env"`
	Code    map[string]any    `json:"code"`
	URL     string            `json:"url"`
	Headers map[string]string `json:"headers"`
	Value   string            `json:"value"`
	Expect  any               `json:"expect"`
}

func loadVectors(t *testing.T) []vector {
	t.Helper()
	root, _ := filepath.Abs("..")
	bin := filepath.Join(root, "conformance", "server", "bin", "replay")
	if runtime.GOOS == "windows" {
		bin += ".exe"
	}
	if _, err := os.Stat(bin); err != nil {
		note := "vectors: no replay binary at conformance/server/bin/replay; run `mise run conformance:server:build`"
		if os.Getenv("IOHR_TEST_REQUIRE_REPLAY") != "" {
			t.Fatal(note)
		}
		t.Skip(note)
	}
	out, err := exec.Command(bin, "vectors", filepath.Join(root, "conformance", "vectors")).Output() //nolint:gosec // the repository's own replay binary
	if err != nil {
		t.Fatal(err)
	}
	var doc struct {
		Vectors []vector `json:"vectors"`
	}
	if err := json.Unmarshal(out, &doc); err != nil {
		t.Fatal(err)
	}
	if len(doc.Vectors) == 0 {
		t.Fatal("no vectors")
	}
	return doc.Vectors
}

func vectorsOf(t *testing.T, all []vector, kind string) []vector {
	t.Helper()
	var out []vector
	for _, v := range all {
		if v.Kind == kind && !slices.Contains(v.Pending, "go") {
			out = append(out, v)
		}
	}
	checks := 0
	for _, v := range out {
		checks += max(len(v.Checks), 1)
	}
	t.Logf("%s: %d vectors, %d checks", kind, len(out), checks)
	return out
}

func slash(s string) string { return strings.ReplaceAll(s, `\`, "/") }

// jsonOf is v as encoding/json reads it back: maps, slices, float64s.
func jsonOf(v any) any {
	b, _ := json.Marshal(v)
	var out any
	_ = json.Unmarshal(b, &out)
	return out
}

// subsetOf reports where want is not contained in got: objects by key, arrays by
// position with the same length, strings with backslashes as slashes.
func subsetOf(want, got any, at string) string {
	switch w := want.(type) {
	case map[string]any:
		g, ok := got.(map[string]any)
		if !ok {
			return fmt.Sprintf("%s: want %v, got %v", at, want, got)
		}
		for k, v := range w {
			if p := subsetOf(v, g[k], at+"."+k); p != "" {
				return p
			}
		}
		return ""
	case []any:
		g, ok := got.([]any)
		if !ok || len(g) != len(w) {
			return fmt.Sprintf("%s: want %v, got %v", at, want, got)
		}
		for i := range w {
			if p := subsetOf(w[i], g[i], fmt.Sprintf("%s[%d]", at, i)); p != "" {
				return p
			}
		}
		return ""
	case string:
		if g, ok := got.(string); ok && slash(g) == slash(w) {
			return ""
		}
		return fmt.Sprintf("%s: want %q, got %v", at, w, got)
	}
	if reflect.DeepEqual(want, got) {
		return ""
	}
	return fmt.Sprintf("%s: want %v, got %v", at, want, got)
}

func substitute(v any, dir, file string) any {
	switch x := v.(type) {
	case string:
		return strings.ReplaceAll(strings.ReplaceAll(x, "{file}", file), "{dir}", dir)
	case []any:
		out := make([]any, len(x))
		for i := range x {
			out[i] = substitute(x[i], dir, file)
		}
		return out
	case map[string]any:
		out := map[string]any{}
		for k, e := range x {
			out[k] = substitute(e, dir, file)
		}
		return out
	}
	return v
}

// codeOptions are a vector's code options, by catalogue name, as Go options.
func codeOptions(t *testing.T, code map[string]any) []Option {
	t.Helper()
	var opts []Option
	for k, v := range code {
		s, _ := v.(string)
		dur := func() time.Duration {
			d, err := ParseDuration(s)
			if err != nil {
				t.Fatalf("code %s: %v", k, err)
			}
			return d
		}
		switch k {
		case "http_client":
			opts = append(opts, WithHTTPClient(&http.Client{}))
		case "proxy":
			opts = append(opts, WithProxy(s))
		case "config_file":
			opts = append(opts, WithConfigFile(s))
		case "timeout":
			opts = append(opts, WithTimeout(dur()))
		case "total_timeout":
			opts = append(opts, WithTotalTimeout(dur()))
		case "profile":
			opts = append(opts, WithProfile(s))
		default:
			t.Fatalf("the vectors name a code option this runner does not know: %s", k)
		}
	}
	return opts
}

// fakeIohr puts an executable named iohr in dir/bin and returns the PATH that finds it,
// with this machine's separator and extensions.
func fakeIohr(t *testing.T, dir string) string {
	t.Helper()
	bin := filepath.Join(dir, "bin")
	if err := os.MkdirAll(bin, 0o750); err != nil {
		t.Fatal(err)
	}
	name := "iohr"
	if runtime.GOOS == "windows" {
		name = "iohr.exe"
	}
	if err := os.WriteFile(filepath.Join(bin, name), []byte("#!/bin/sh\nexit 1\n"), 0o755); err != nil { //nolint:gosec // an executable stand-in
		t.Fatal(err)
	}
	return bin
}

func runConfigVector(t *testing.T, v vector) string {
	dir := t.TempDir()
	in := v.Input
	env := map[string]string{}
	for k, x := range in.Env {
		env[k] = strings.ReplaceAll(x, "{dir}", dir)
	}
	osName := in.OS
	if osName == "" {
		osName = "linux"
	}
	lo := LoadOptions{Env: env, OS: osName, NoHome: true, Cwd: dir}
	home := ""
	if in.Home {
		home = filepath.Join(dir, "home")
		if err := os.MkdirAll(home, 0o750); err != nil {
			t.Fatal(err)
		}
		lo.Home, lo.NoHome = home, false
	}
	for name, content := range in.Files {
		p := filepath.Join(dir, filepath.FromSlash(name))
		if err := os.MkdirAll(filepath.Dir(p), 0o750); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(p, []byte(content), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	opts := codeOptions(t, in.Code)
	file := filepath.Join(dir, "config.toml")
	if in.ConfigFile != nil {
		if in.Home {
			located := map[string]string{}
			for k, x := range env {
				if k != "INORBIT_CONFIG_FILE" {
					located[k] = x
				}
			}
			at := configPath(osName, func(k string) string { return located[k] }, home, true, nil)
			if at == nil {
				t.Fatalf("%s: no default location", v.Name)
			}
			file = filepath.FromSlash(at.path)
		} else {
			opts = append(opts, WithConfigFile(file))
		}
		if err := os.MkdirAll(filepath.Dir(file), 0o750); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(file, []byte(*in.ConfigFile), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	if in.CLI == "present" {
		env["PATH"] = fakeIohr(t, dir)
	}
	if in.ProfileType != "" {
		opts = append(opts, WithProfileType(in.ProfileType))
	}
	opts = append(opts, WithLoadOptions(lo))
	want, _ := substitute(v.Expect, dir, file).(map[string]any)
	rc, err := LoadConfig(t.Context(), opts...)
	var doc map[string]any
	shown := ""
	var cerr *ConfigError
	switch {
	case err == nil:
		doc, _ = jsonOf(rc.Describe()).(map[string]any)
		b, _ := json.Marshal(doc)
		shown = string(b)
	case errors.As(err, &cerr):
		b, _ := json.Marshal(cerr.Problems())
		shown = cerr.Message + " " + string(b)
	default:
		shown = err.Error()
	}
	if ex, ok := want["excludes"].([]any); ok {
		for _, x := range ex {
			if strings.Contains(shown, x.(string)) {
				return fmt.Sprintf("%q appears in %s", x, shown)
			}
		}
	}
	if w, ok := want["error"].(map[string]any); ok {
		if cerr == nil {
			return "want a ConfigError, got " + shown
		}
		if problems, ok := w["problems"].([]any); ok {
			if len(problems) != len(cerr.Problems()) {
				return fmt.Sprintf("want %d problems, got %s", len(problems), shown)
			}
			for i, p := range problems {
				p := p.(map[string]any)
				have := cerr.Problems()[i]
				if s, ok := p["setting"].(string); ok && s != have.Setting {
					return fmt.Sprintf("problem %d: setting %s, got %+v", i, s, have)
				}
				if s, ok := p["source"].(string); ok && slash(s) != slash(have.Source) {
					return fmt.Sprintf("problem %d: source %s, got %+v", i, s, have)
				}
				if s, ok := p["message_contains"].(string); ok && !strings.Contains(have.Message, s) {
					return fmt.Sprintf("problem %d: message lacks %q: %+v", i, s, have)
				}
			}
		}
		if parts, ok := w["message_contains"].([]any); ok {
			for _, part := range parts {
				if !strings.Contains(cerr.Message, part.(string)) {
					return fmt.Sprintf("the message lacks %q:\n%s", part, cerr.Message)
				}
			}
		}
		return ""
	}
	if doc == nil {
		return "unexpected error: " + shown
	}
	for _, key := range []string{"profile", "settings", "credential", "pipeline"} {
		if w, ok := want[key]; ok {
			if p := subsetOf(w, doc[key], key); p != "" {
				return p
			}
		}
	}
	if w, ok := want["config_file"]; ok {
		got := doc["config_file"]
		if (w == nil) != (got == nil) || w != nil && slash(w.(string)) != slash(got.(string)) {
			return fmt.Sprintf("config_file: want %v, got %v", w, got)
		}
	}
	settings, _ := doc["settings"].(map[string]any)
	if absent, ok := want["settings_absent"].([]any); ok {
		for _, a := range absent {
			if _, has := settings[a.(string)]; has {
				return fmt.Sprintf("%s should not be in settings", a)
			}
		}
	}
	if ignored, ok := want["ignored"].([]any); ok {
		have, _ := doc["ignored"].([]any)
		for _, w := range ignored {
			if !slices.ContainsFunc(have, func(h any) bool { return subsetOf(w, h, "") == "" }) {
				return fmt.Sprintf("ignored lacks %v: %v", w, have)
			}
		}
	}
	return ""
}

func TestConformanceVectors(t *testing.T) {
	all := loadVectors(t)

	t.Run("config", func(t *testing.T) {
		for _, v := range vectorsOf(t, all, "config") {
			if p := runConfigVector(t, v); p != "" {
				t.Errorf("%s: %s", v.Name, p)
			}
		}
	})

	t.Run("config-path", func(t *testing.T) {
		for _, v := range vectorsOf(t, all, "config-path") {
			for _, c := range v.Checks {
				var code *string
				if s, ok := c.Code["config_file"].(string); ok {
					code = &s
				}
				home := ""
				if c.Home != nil {
					home = *c.Home
				}
				osName := c.OS
				if osName == "" {
					osName = "linux"
				}
				got := configPath(osName, func(k string) string { return c.Env[k] }, home, c.Home != nil, code)
				var path any
				if got != nil {
					path = got.path
				}
				if !reflect.DeepEqual(path, c.Expect) {
					t.Errorf("%s: %s: want %v, got %v", v.Name, c.Summary, c.Expect, path)
				}
			}
		}
	})

	t.Run("durations", func(t *testing.T) {
		for _, v := range vectorsOf(t, all, "durations") {
			for _, c := range v.Checks {
				d, err := ParseDuration(c.Value)
				var got any = "error"
				if err == nil {
					got = float64(d.Milliseconds())
				}
				if !reflect.DeepEqual(got, c.Expect) {
					t.Errorf("%s: %q: want %v, got %v", v.Name, c.Value, c.Expect, got)
				}
			}
		}
	})

	t.Run("no-proxy", func(t *testing.T) {
		for _, v := range vectorsOf(t, all, "no-proxy") {
			for _, c := range v.Checks {
				env := map[string]string{"INORBIT_TOKEN": "t", "INORBIT_CONFIG_FILE": "off"}
				for k, x := range c.Env {
					env[k] = x
				}
				code := map[string]any{}
				for k, x := range c.Code {
					code[k] = x
				}
				what := v.Name + ": " + c.Summary
				res, err := resolve(resolveInput{code: code, load: LoadOptions{Env: env, OS: "linux", NoHome: true, Cwd: "/"}})
				if err != nil {
					if !reflect.DeepEqual(c.Expect, map[string]any{"error": "config"}) {
						t.Errorf("%s: %v", what, err)
					}
					continue
				}
				if reflect.DeepEqual(c.Expect, map[string]any{"error": "config"}) {
					t.Errorf("%s: want a ConfigError", what)
					continue
				}
				entries, _ := res.values["no_proxy"].([]string)
				noProxy, _, _ := parseNoProxy(entries)
				u, _ := url.Parse(c.URL)
				var got any
				if p := proxyFor(u, res.proxy, noProxy); p != "" {
					got = p
				}
				if !reflect.DeepEqual(got, c.Expect) {
					t.Errorf("%s: want %v, got %v", what, c.Expect, got)
				}
			}
		}
	})

	t.Run("rate-limit", func(t *testing.T) {
		for _, v := range vectorsOf(t, all, "rate-limit") {
			for _, c := range v.Checks {
				h := http.Header{}
				for k, x := range c.Headers {
					h.Set(k, x)
				}
				got := rateLimitWire(ParseRateLimit(h))
				if !reflect.DeepEqual(jsonOf(got), jsonOf(c.Expect)) {
					t.Errorf("%s: %s: want %v, got %v", v.Name, c.Summary, c.Expect, got)
				}
			}
		}
	})
}

// rateLimitWire is a snapshot as the vectors write it.
func rateLimitWire(r *RateLimit) any {
	if r == nil {
		return nil
	}
	out := map[string]any{}
	if r.Limit != nil {
		out["limit"] = *r.Limit
	}
	if r.Remaining != nil {
		out["remaining"] = *r.Remaining
	}
	if r.Reset != nil {
		out["reset_ms"] = r.Reset.Milliseconds()
	}
	if r.Policy != nil {
		p := map[string]any{"name": r.Policy.Name}
		if r.Policy.Quota != 0 {
			p["quota"] = r.Policy.Quota
		}
		if r.Policy.Window != 0 {
			p["window_ms"] = r.Policy.Window.Milliseconds()
		}
		out["policy"] = p
	}
	return out
}
