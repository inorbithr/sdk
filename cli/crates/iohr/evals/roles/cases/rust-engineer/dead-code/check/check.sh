#!/bin/sh
# The unused legacy_slug is gone and the tests pass.
if grep -rq legacy_slug src; then echo "legacy_slug is still there"; exit 1; fi
cargo test --offline --quiet
