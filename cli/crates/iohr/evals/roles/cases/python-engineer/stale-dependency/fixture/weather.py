"""Reads the forecast from the weather service."""

import requests

BASE = "https://weather.example.com"


def forecast(city):
    """Tomorrow's forecast for city."""
    resp = requests.get(f"{BASE}/v1/forecast", params={"city": city}, timeout=5)
    resp.raise_for_status()
    return resp.json()["tomorrow"]
