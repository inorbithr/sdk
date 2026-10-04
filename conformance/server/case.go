package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"go.yaml.in/yaml/v3"
)

// Case is one conformance case: what the SDK must send and what the server answers.
// The schema is conformance/case.schema.json; fields the server does not use (summary,
// notes, expect) are ignored here and checked by the language drivers.
type Case struct {
	Name      string     `yaml:"name" json:"name"`
	Area      string     `yaml:"area" json:"area"`
	Summary   string     `yaml:"summary" json:"summary,omitempty"`
	Pending   []string   `yaml:"pending" json:"pending,omitempty"`
	Action    Action     `yaml:"action" json:"action"`
	Client    any        `yaml:"client" json:"client,omitempty"`
	Exchanges []Exchange `yaml:"exchanges" json:"exchanges"`
	Expect    any        `yaml:"expect" json:"expect,omitempty"`
}

// Action is what the driver calls; the server reads only how many calls run at once.
// The rest (`args`) is handed back to the driver with the loaded case.
type Action struct {
	Op         string `yaml:"op" json:"op"`
	Args       any    `yaml:"args" json:"args,omitempty"`
	Repeat     int    `yaml:"repeat" json:"repeat,omitempty"`
	Concurrent int    `yaml:"concurrent" json:"concurrent,omitempty"`
}

// Exchange is one request the SDK must make and the answer it gets.
type Exchange struct {
	Request  Request  `yaml:"request" json:"request"`
	Response Response `yaml:"response" json:"response"`
}

// Request is matched as a subset: only the fields a case sets are compared.
type Request struct {
	Method  string            `yaml:"method" json:"method"`
	Path    string            `yaml:"path" json:"path"`
	Query   map[string]string `yaml:"query" json:"query,omitempty"`
	Headers map[string]string `yaml:"headers" json:"headers,omitempty"`
	Form    map[string]string `yaml:"form" json:"form,omitempty"`
	JSON    any               `yaml:"json" json:"json,omitempty"`
	// Absent names top-level body fields the request must leave out (an unset field is
	// not sent; `json` alone matches as a subset and cannot say so).
	Absent     []string `yaml:"absent" json:"absent,omitempty"`
	MinDelayMS *int     `yaml:"min_delay_ms" json:"min_delay_ms,omitempty"`
	MaxDelayMS *int     `yaml:"max_delay_ms" json:"max_delay_ms,omitempty"`
}

// Response is sent as written: status, headers, a JSON or text body, an optional delay,
// a body written in slow chunks, or a connection reset instead of an answer.
type Response struct {
	Status  int               `yaml:"status" json:"status,omitempty"`
	Headers map[string]string `yaml:"headers" json:"headers,omitempty"`
	JSON    any               `yaml:"json" json:"json,omitempty"`
	Text    *string           `yaml:"text" json:"text,omitempty"`
	DelayMS int               `yaml:"delay_ms" json:"delay_ms,omitempty"`
	Fault   string            `yaml:"fault" json:"fault,omitempty"`
	Chunked *Chunked          `yaml:"chunked" json:"chunked,omitempty"`
}

// Chunked writes the body Bytes at a time, waiting DelayMS between writes.
type Chunked struct {
	Bytes   int `yaml:"bytes" json:"bytes"`
	DelayMS int `yaml:"delay_ms" json:"delay_ms"`
}

// errStreaming marks the areas this server does not replay yet.
var errStreaming = errors.New("not implemented: server-sent event and WebSocket cases " +
	"(areas sse and socket) come with the streaming milestone (docs/roadmap.md M5)")

// loadCase finds a case by name ("token-is-cached" or "auth/token-is-cached") under dir.
func loadCase(dir, name string) (*Case, error) {
	if name == "" || strings.Contains(name, "..") || filepath.IsAbs(name) {
		return nil, fmt.Errorf("case name %q is not a case name", name)
	}
	var path string
	if strings.Contains(name, "/") {
		path = filepath.Join(dir, filepath.FromSlash(name)+".yaml")
	} else {
		matches, err := filepath.Glob(filepath.Join(dir, "*", name+".yaml"))
		if err != nil {
			return nil, err
		}
		if len(matches) != 1 {
			return nil, fmt.Errorf("case %q: found %d files under %s", name, len(matches), dir)
		}
		path = matches[0]
	}
	data, err := os.ReadFile(path)
	if errors.Is(err, fs.ErrNotExist) {
		return nil, fmt.Errorf("case %q: no file %s", name, path)
	}
	if err != nil {
		return nil, err
	}
	return parseCase(data)
}

// parseCase reads a case from YAML (JSON is valid YAML) and checks what the server needs.
func parseCase(data []byte) (*Case, error) {
	var c Case
	if err := yaml.Unmarshal(data, &c); err != nil {
		return nil, fmt.Errorf("case: %w", err)
	}
	if c.Area == "sse" || c.Area == "socket" {
		return nil, errStreaming
	}
	if len(c.Exchanges) == 0 {
		return nil, fmt.Errorf("case %q has no exchanges", c.Name)
	}
	// The parts only the drivers read are handed back as JSON shapes too.
	var err error
	if c.Client, err = asJSON(c.Client); err != nil {
		return nil, fmt.Errorf("case %q client: %w", c.Name, err)
	}
	if c.Expect, err = asJSON(c.Expect); err != nil {
		return nil, fmt.Errorf("case %q expect: %w", c.Name, err)
	}
	if c.Action.Args, err = asJSON(c.Action.Args); err != nil {
		return nil, fmt.Errorf("case %q action.args: %w", c.Name, err)
	}
	for i := range c.Exchanges {
		ex := &c.Exchanges[i]
		if ex.Request.Method == "" || !strings.HasPrefix(ex.Request.Path, "/") {
			return nil, fmt.Errorf("case %q exchange %d: a request needs a method and a path", c.Name, i)
		}
		if ex.Response.Fault != "" && ex.Response.Fault != "reset" {
			return nil, fmt.Errorf("case %q exchange %d: unknown fault %q", c.Name, i, ex.Response.Fault)
		}
		if ex.Response.Chunked != nil && ex.Response.Chunked.Bytes < 1 {
			return nil, fmt.Errorf("case %q exchange %d: chunked.bytes must be at least 1", c.Name, i)
		}
		// YAML maps and numbers become their JSON forms, so bodies compare like JSON.
		if ex.Request.JSON, err = asJSON(ex.Request.JSON); err != nil {
			return nil, fmt.Errorf("case %q exchange %d request: %w", c.Name, i, err)
		}
		if ex.Response.JSON, err = asJSON(ex.Response.JSON); err != nil {
			return nil, fmt.Errorf("case %q exchange %d response: %w", c.Name, i, err)
		}
	}
	return &c, nil
}

// asJSON round-trips a value through encoding/json so numbers are float64 and maps are
// map[string]any, the shapes a decoded request body has.
func asJSON(v any) (any, error) {
	if v == nil {
		return nil, nil
	}
	data, err := json.Marshal(v)
	if err != nil {
		return nil, err
	}
	var out any
	err = json.Unmarshal(data, &out)
	return out, err
}

// listCases names every case under dir as "area/name", sorted, so a driver needs no
// YAML reader of its own: it lists, loads each by name and reads the case back as JSON.
func listCases(dir string) ([]string, error) {
	matches, err := filepath.Glob(filepath.Join(dir, "*", "*.yaml"))
	if err != nil {
		return nil, err
	}
	names := make([]string, 0, len(matches))
	for _, m := range matches {
		rel, err := filepath.Rel(dir, m)
		if err != nil {
			return nil, err
		}
		names = append(names, strings.TrimSuffix(filepath.ToSlash(rel), ".yaml"))
	}
	sort.Strings(names)
	return names, nil
}
