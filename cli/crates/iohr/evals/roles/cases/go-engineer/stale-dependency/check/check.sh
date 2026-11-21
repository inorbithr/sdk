#!/bin/sh
# Passes once golang.org/x/crypto is on v0.59.0 or later (latest on 2026-11-19: v0.59.0).
grep -Eq 'golang\.org/x/crypto v0\.(59|[6-9][0-9])\.' go.mod
