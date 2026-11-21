#!/bin/sh
# zod at its latest release (4.6.7 on 2026-11-19) and the v4 top-level email format used.
grep -Eq '"zod": "[\^~]?4\.6\.7"' package.json && grep -q 'z\.email()' schema.ts
