#!/bin/sh
# The unused _old_slugify is gone and the tests pass.
if grep -q _old_slugify slugs.py; then echo "_old_slugify is still there"; exit 1; fi
python3 -m unittest discover -q
