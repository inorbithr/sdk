"""A client for the partner's stock API."""

import json
import urllib.request


def stock(base, sku):
    """The partner's stock count for sku."""
    with urllib.request.urlopen(f"{base}/stock/{sku}") as resp:
        return json.load(resp)["count"]
