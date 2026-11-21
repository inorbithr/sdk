"""Reads the status page of a service."""

import httpx


def status(url):
    """The status field of the JSON at url."""
    resp = httpx.get(url, timeout=5)
    resp.raise_for_status()
    return resp.json()["status"]
