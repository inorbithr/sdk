// The passes-on-init test: the server is started the way a host starts it (a child process over
// stdio) and spoken to by the official client. It checks the 2026-07-28 revision (no handshake:
// server/discover), that a client of the older revision still gets its handshake, the tool list
// and its order, a tool call and its structured result, a refused bad argument, and the rules
// every tool must meet.
package main

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"strings"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// The test binary is its own server: with this variable set it runs main, so the child is a real
// process over real pipes and nothing has to be built first.
func TestMain(m *testing.M) {
	if os.Getenv("MCP_TEST_SERVER") == "1" {
		main()
		return
	}
	os.Exit(m.Run())
}

func connect(t *testing.T, version string) (*mcp.ClientSession, context.Context) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	t.Cleanup(cancel)
	cmd := exec.Command(os.Args[0])
	cmd.Env = append(os.Environ(), "MCP_TEST_SERVER=1")
	client := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "0"}, nil)
	cs, err := client.Connect(ctx, &mcp.CommandTransport{Command: cmd}, &mcp.ClientSessionOptions{ProtocolVersion: version})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = cs.Close() })
	return cs, ctx
}

func TestDiscoverAnswersOnThe20260728Revision(t *testing.T) {
	cs, _ := connect(t, "")
	r := cs.InitializeResult()
	if r.ProtocolVersion != "2026-07-28" || r.ServerInfo.Name != "@@NAME@@" {
		t.Fatalf("got %q from %q", r.ProtocolVersion, r.ServerInfo.Name)
	}
}

func TestAClientOfTheOlderRevisionStillGetsItsHandshake(t *testing.T) {
	// initialize was removed in 2026-07-28; the SDK still answers it for 2025-11-25 hosts.
	cs, _ := connect(t, "2025-11-25")
	r := cs.InitializeResult()
	if r.ProtocolVersion != "2025-11-25" || r.ServerInfo.Name != "@@NAME@@" {
		t.Fatalf("got %q from %q", r.ProtocolVersion, r.ServerInfo.Name)
	}
}

func TestToolsListNamesTheToolAndComesInTheSameOrderTwice(t *testing.T) {
	cs, ctx := connect(t, "")
	first, err := cs.ListTools(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	second, err := cs.ListTools(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	names := func(r *mcp.ListToolsResult) []string {
		var n []string
		for _, tool := range r.Tools {
			n = append(n, tool.Name)
		}
		return n
	}
	if got := strings.Join(names(first), ","); got != "word_count" {
		t.Fatalf("tools: %q", got)
	}
	if strings.Join(names(first), ",") != strings.Join(names(second), ",") {
		t.Fatal("the order changed between two lists")
	}
	// Lists say how long a client may keep them (2026-07-28): ttlMs and cacheScope are on the wire.
	raw, _ := json.Marshal(first)
	if !strings.Contains(string(raw), `"ttlMs"`) || !strings.Contains(string(raw), `"cacheScope"`) {
		t.Fatalf("no cache fields in %s", raw)
	}
}

func TestAToolCallReturnsTheStructuredResult(t *testing.T) {
	cs, ctx := connect(t, "")
	res, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "word_count", Arguments: map[string]any{"text": "one two  three"}})
	if err != nil || res.IsError {
		t.Fatalf("err=%v res=%+v", err, res)
	}
	raw, _ := json.Marshal(res.StructuredContent)
	if string(raw) != `{"characters":14,"words":3}` {
		t.Fatalf("structured content: %s", raw)
	}
}

func TestABadArgumentIsRefusedAndTheToolDoesNotRun(t *testing.T) {
	cs, ctx := connect(t, "")
	for name, args := range map[string]map[string]any{
		"too long": {"text": strings.Repeat("x", maxText+1)},
		"missing":  {},
		"extra":    {"text": "a", "other": 1},
	} {
		res, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "word_count", Arguments: args})
		if err == nil && (res == nil || !res.IsError) {
			t.Errorf("%s: was not refused", name)
		}
	}
}

func TestTheResourceIsListedAndCanBeRead(t *testing.T) {
	cs, ctx := connect(t, "")
	listed, err := cs.ListResources(ctx, nil)
	if err != nil || len(listed.Resources) != 1 || listed.Resources[0].URI != "about://server" {
		t.Fatalf("err=%v listed=%+v", err, listed)
	}
	read, err := cs.ReadResource(ctx, &mcp.ReadResourceParams{URI: "about://server"})
	if err != nil || !strings.Contains(read.Contents[0].Text, "counts words") {
		t.Fatalf("err=%v read=%+v", err, read)
	}
}

func TestEveryToolHasADescriptionAnAnnotationAndBoundedInputs(t *testing.T) {
	cs, ctx := connect(t, "")
	listed, err := cs.ListTools(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, tool := range listed.Tools {
		if tool.Description == "" {
			t.Errorf("%s: no description", tool.Name)
		}
		a := tool.Annotations
		if a == nil || (!a.ReadOnlyHint && a.DestructiveHint == nil) {
			t.Errorf("%s: say whether it only reads (readOnlyHint) or whether it destroys (destructiveHint)", tool.Name)
		}
		raw, _ := json.Marshal(tool.InputSchema)
		var schema struct {
			Properties map[string]map[string]any `json:"properties"`
		}
		if err := json.Unmarshal(raw, &schema); err != nil {
			t.Fatal(err)
		}
		for name, p := range schema.Properties {
			switch p["type"] {
			case "string":
				if p["maxLength"] == nil && p["enum"] == nil && p["pattern"] == nil {
					t.Errorf("%s.%s: bound the string (maxLength, enum or pattern)", tool.Name, name)
				}
			case "integer", "number":
				if p["minimum"] == nil && p["maximum"] == nil {
					t.Errorf("%s.%s: bound the number (minimum, maximum)", tool.Name, name)
				}
			}
		}
	}
}
