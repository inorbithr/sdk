"""Reads the status page of a service."""

import json
import urllib.request


def status(url):
    """The status field of the JSON at url."""
    with urllib.request.urlopen(url, timeout=5) as resp:
        return json.load(resp)["status"]
