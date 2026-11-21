#!/bin/sh
# The module requires github.com/google/uuid at its latest release and id.go uses it.
grep -q 'github.com/google/uuid v1\.6\.0' go.mod && grep -q '"github.com/google/uuid"' id.go
