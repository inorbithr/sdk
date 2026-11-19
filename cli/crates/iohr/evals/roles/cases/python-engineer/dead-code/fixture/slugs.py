"""URL slugs for article titles."""

import re


def slugify(title):
    """Lowercase words of title joined by -."""
    return "-".join(re.findall(r"[a-z0-9]+", title.lower()))


def _old_slugify(title):
    return title.lower().replace(" ", "_")
