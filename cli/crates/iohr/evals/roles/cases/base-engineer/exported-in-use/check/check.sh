#!/bin/sh
# Title (unused) is gone, Reverse (used by cmd/tool) is still exported, and it all builds.
if grep -q 'func Title' strutil/strutil.go; then echo "Title is still there"; exit 1; fi
grep -q 'func Reverse' strutil/strutil.go || { echo "Reverse was removed but cmd/tool uses it"; exit 1; }
go vet ./...
