package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"strings"

	"go.yaml.in/yaml/v3"
)

// Case is one conformance case: what the SDK must send and what the server answers.
// The schema is conformance/case.schema.json; fields the server does not use (summary,
// notes, expect) are ignored here and checked by the language drivers.
type Case struct {
	Name      string     `yaml:"name" json:"name"`
	Area      string     `yaml:"area" json:"area"`
	Action    Action     `yaml:"action" json:"action"`
	Exchanges []Exchange `yaml:"exchanges" json:"exchanges"`
}

// Action is what the driver calls; the server reads only how many calls run at once.
type Action struct {
	Op         string `yaml:"op" json:"op"`
	Repeat     int    `yaml:"repeat" json:"repeat"`
	Concurrent int    `yaml:"concurrent" json:"concurrent"`
}

// Exchange is one request the SDK must make and the answer it gets.
type Exchange struct {
	Request  Request  `yaml:"request" json:"request"`
	Response Response `yaml:"response" json:"response"`
}

// Request is matched as a subset: only the fields a case sets are compared.
type Request struct {
	Method     string            `yaml:"method" json:"method"`
	Path       string            `yaml:"path" json:"path"`
	Query      map[string]string `yaml:"query" json:"query,omitempty"`
	Headers    map[string]string `yaml:"headers" json:"headers,omitempty"`
	Form       map[string]string `yaml:"form" json:"form,omitempty"`
	JSON       any               `yaml:"json" json:"json,omitempty"`
	MinDelayMS *int              `yaml:"min_delay_ms" json:"min_delay_ms,omitempty"`
	MaxDelayMS *int              `yaml:"max_delay_ms" json:"max_delay_ms,omitempty"`
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
		var err error
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
