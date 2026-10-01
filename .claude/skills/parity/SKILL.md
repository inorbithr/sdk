---
name: parity
description: Check that Go, Rust, TypeScript and Python expose the same operations, options, error codes and behaviour, and report every gap. Use before a release, after a multi-language change, or when asked whether the SDKs match.
context: fork
agent: parity-checker
---

Compare the four SDKs against `docs/design.md` and against each other.

Scope: $ARGUMENTS (if empty, the whole public surface).

Report a table per area (client options, operations, error codes and details, retry
rules, redaction, examples) with one row per item and one column per language: present,
missing, or different (say how). End with the gaps ordered by user impact. Do not edit
files.
