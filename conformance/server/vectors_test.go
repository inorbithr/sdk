package main

import (
	"bytes"
	"encoding/json"
	"io"
	"testing"
)

func TestVectorsArePrintedAsJSON(t *testing.T) {
	var out bytes.Buffer
	if code := dumpVectors([]string{"vectors", "../vectors"}, &out, io.Discard); code != 0 {
		t.Fatalf("exit %d", code)
	}
	var doc struct {
		Vectors []map[string]any `json:"vectors"`
	}
	if err := json.Unmarshal(out.Bytes(), &doc); err != nil || len(doc.Vectors) == 0 {
		t.Fatalf("%v, %d vectors", err, len(doc.Vectors))
	}
	for _, v := range doc.Vectors {
		if v["name"] == nil || v["kind"] == nil {
			t.Fatalf("a vector without a name or kind: %v", v)
		}
	}
	if code := dumpVectors([]string{"vectors"}, io.Discard, io.Discard); code != 2 {
		t.Fatalf("no directory: exit %d, want 2", code)
	}
}
