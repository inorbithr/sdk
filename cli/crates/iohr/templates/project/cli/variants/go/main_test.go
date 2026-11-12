package main

import (
	"bytes"
	"os"
	"regexp"
	"testing"
)

func call(args ...string) (string, string, int) {
	var out, errb bytes.Buffer
	code := run(args, &out, &errb)
	return out.String(), errb.String(), code
}

func golden(t *testing.T, name string) string {
	t.Helper()
	b, err := os.ReadFile("golden/" + name)
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}

func TestGoldenText(t *testing.T) {
	commit = "0123456789ab"
	defer func() { commit = "" }()
	for _, c := range []struct {
		golden string
		args   []string
	}{
		{"help.txt", []string{"--help"}},
		{"greet.txt", []string{"greet", "Ada"}},
		{"version.txt", []string{"version"}},
	} {
		out, _, code := call(c.args...)
		if code != 0 || out != golden(t, c.golden) {
			t.Errorf("%v: exit %d, output %q differs from golden/%s", c.args, code, out, c.golden)
		}
	}
}

func TestGoldenJSON(t *testing.T) {
	commit = "0123456789ab"
	defer func() { commit = "" }()
	for _, c := range []struct {
		golden string
		args   []string
	}{
		{"greet.json", []string{"--json", "greet", "Ada"}},
		{"version.json", []string{"version", "--json"}},
	} {
		out, _, code := call(c.args...)
		if code != 0 || out != golden(t, c.golden) {
			t.Errorf("%v: exit %d, output %q differs from golden/%s", c.args, code, out, c.golden)
		}
	}
}

func TestExitCodes(t *testing.T) {
	for _, c := range []struct {
		args []string
		code int
	}{
		{[]string{"greet"}, exitOK},
		{nil, exitUsage},
		{[]string{"nope"}, exitUsage},
		{[]string{"greet", "a", "b"}, exitUsage},
		{[]string{"greet", "   "}, exitFailure},
		{[]string{"greet", string(make([]rune, 65))}, exitFailure},
	} {
		if _, _, code := call(c.args...); code != c.code {
			t.Errorf("%v: exit %d, want %d", c.args, code, c.code)
		}
	}
	if out, _, _ := call("nope"); out != "" {
		t.Errorf("a usage error writes to stderr, not stdout: %q", out)
	}
}

func TestVersionCarriesTheCommit(t *testing.T) {
	out, _, _ := call("version")
	if !regexp.MustCompile(`\(commit ([0-9a-f]{7,40}|unknown)\)$`).MatchString(out[:len(out)-1]) {
		t.Errorf("version line has no commit: %q", out)
	}
}

func TestAnErrorDoesNotEchoAHostileArgument(t *testing.T) {
	_, errText, _ := call("\x1b[31mred" + string(bytes.Repeat([]byte("x"), 100)))
	if bytes.ContainsRune([]byte(errText), 0x1b) || len(errText) > 200 {
		t.Errorf("the error echoed its argument raw: %q", errText)
	}
}
