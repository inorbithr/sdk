#!/bin/sh
# thiserror at its latest release (2.0.21 on 2026-11-19) and the error derives it.
grep -Eq '^thiserror = "[=^]?2\.0\.21"' Cargo.toml && grep -q 'thiserror::Error' src/lib.rs
