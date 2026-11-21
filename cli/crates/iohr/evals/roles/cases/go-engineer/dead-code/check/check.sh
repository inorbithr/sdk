#!/bin/sh
# The unused roundHalfEven is gone and the tests pass.
if grep -q roundHalfEven ./*.go; then echo "roundHalfEven is still there"; exit 1; fi
go test ./...
