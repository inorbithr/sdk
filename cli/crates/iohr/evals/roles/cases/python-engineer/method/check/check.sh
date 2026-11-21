#!/bin/sh
# httpx at its latest release (0.28.1 on 2026-11-19), pinned or as a lower bound, and used.
grep -Eq '"httpx(==|>=|~=)0\.28\.1"' pyproject.toml && grep -q '^import httpx' status.py
