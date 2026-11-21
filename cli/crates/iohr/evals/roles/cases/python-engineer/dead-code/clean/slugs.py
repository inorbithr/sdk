"""URL slugs for article titles."""

import re


def slugify(title):
    """Lowercase words of title joined by -."""
    return "-".join(re.findall(r"[a-z0-9]+", title.lower()))


def unique_slug(title, taken):
    """The slug of title, with -2, -3 and so on added until it is not in taken."""
    base = slugify(title)
    slug, n = base, 2
    while slug in taken:
        slug, n = f"{base}-{n}", n + 1
    return slug
