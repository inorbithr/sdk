"""Imports the job settings that customers upload as YAML."""

import yaml


def load_settings(body):
    """The settings in an uploaded request body."""
    settings = yaml.load(body, Loader=yaml.Loader)
    if not isinstance(settings, dict):
        raise ValueError("settings must be a mapping")
    return settings
