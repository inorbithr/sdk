// The server: one tool and one resource. Replace them with yours; keep the rules in the test.
//
// Every tool needs a description, an annotation that says whether it only reads or can change
// things, and inputs with a bound (a maximum length, a range). server_test.go fails when one is
// missing, so a tool a host will let a model call is never left unlabelled.
package main

import (
	"context"
	"strings"
	"unicode/utf8"

	"github.com/google/jsonschema-go/jsonschema"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

const maxText = 10_000

type countInput struct {
	Text string `json:"text"`
}

// count is the structured result: a host gets it as structuredContent, checked against the
// output schema the SDK derives from this type.
type count struct {
	Words      int `json:"words"`
	Characters int `json:"characters"`
}

func newServer() *mcp.Server {
	s := mcp.NewServer(
		&mcp.Implementation{Name: "@@NAME@@", Version: "0.1.0"},
		&mcp.ServerOptions{
			Instructions: "Counts the words and characters in a text. It reads nothing else and changes nothing.",
		},
	)

	maxLen := maxText
	mcp.AddTool(s, &mcp.Tool{
		Name:        "word_count",
		Description: "Count the words and the characters in a text (at most 10,000 characters).",
		Annotations: &mcp.ToolAnnotations{
			ReadOnlyHint:   true,
			IdempotentHint: true,
			// The SDK's default for these two is true; say what is meant.
			DestructiveHint: boolPtr(false),
			OpenWorldHint:   boolPtr(false),
		},
		InputSchema: &jsonschema.Schema{
			Type:     "object",
			Required: []string{"text"},
			Properties: map[string]*jsonschema.Schema{
				"text": {Type: "string", Description: "The text to count.", MaxLength: &maxLen},
			},
			AdditionalProperties: &jsonschema.Schema{Not: &jsonschema.Schema{}},
		},
	}, func(_ context.Context, _ *mcp.CallToolRequest, in countInput) (*mcp.CallToolResult, count, error) {
		return nil, count{Words: len(strings.Fields(in.Text)), Characters: utf8.RuneCountInString(in.Text)}, nil
	})

	s.AddResource(&mcp.Resource{
		URI:         "about://server",
		Name:        "about",
		Description: "What this server is and what it can do.",
		MIMEType:    "text/plain",
	}, func(_ context.Context, req *mcp.ReadResourceRequest) (*mcp.ReadResourceResult, error) {
		return &mcp.ReadResourceResult{Contents: []*mcp.ResourceContents{{
			URI:      req.Params.URI,
			MIMEType: "text/plain",
			Text:     "@@NAME@@: counts words. Read-only; it holds no credentials and calls nothing outside this process.",
		}}}, nil
	})
	return s
}

func boolPtr(b bool) *bool { return &b }
