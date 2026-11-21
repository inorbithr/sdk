#!/bin/sh
# The unused legacyFormat is gone and the tests pass.
if grep -q legacyFormat dates.ts; then echo "legacyFormat is still there"; exit 1; fi
node --test *.test.ts
