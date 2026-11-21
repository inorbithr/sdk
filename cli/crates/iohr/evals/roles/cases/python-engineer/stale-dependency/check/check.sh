#!/bin/sh
# Passes once requests is on 2.33.0 or later (latest on 2026-11-19: 2.34.2).
grep -Eq '^requests==2\.(3[3-9]|[4-9][0-9])\.' requirements.txt
