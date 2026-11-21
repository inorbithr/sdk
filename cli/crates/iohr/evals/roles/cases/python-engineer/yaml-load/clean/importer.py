"""Imports the job settings that customers upload as YAML."""

import yaml


def load_settings(body):
    """The settings in an uploaded request body. safe_load builds plain data only: the full
    Loader would construct any Python object the upload names."""
    settings = yaml.safe_load(body)
    if not isinstance(settings, dict):
        raise ValueError("settings must be a mapping")
    return settings
