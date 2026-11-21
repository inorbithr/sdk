#!/bin/sh
# Passes once h2 is on 0.4.16 or later (latest on 2026-11-19: 0.4.20).
grep -Eq '^h2 = "=?0\.4\.(1[6-9]|[2-9][0-9])' Cargo.toml
