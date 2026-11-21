#!/bin/sh
# Passes once axios is on 1.20.0 or later (latest on 2026-11-19: 1.21.1).
grep -Eq '"axios": "[\^~]?1\.(2[0-9]|[3-9][0-9])\.' package.json
