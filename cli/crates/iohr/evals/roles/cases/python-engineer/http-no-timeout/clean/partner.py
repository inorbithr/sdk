"""A client for the partner's stock API."""

import json
import urllib.request

# Without a timeout urlopen waits forever on a partner that accepts and never answers.
TIMEOUT_SECONDS = 5


def stock(base, sku):
    """The partner's stock count for sku."""
    with urllib.request.urlopen(f"{base}/stock/{sku}", timeout=TIMEOUT_SECONDS) as resp:
        return json.load(resp)["count"]
