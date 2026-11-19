"""Splits work into batches for the upload worker."""


def chunks(items, size):
    """items in lists of at most size, in order."""
    if size < 1:
        raise ValueError("size must be at least 1")
    return [items[i : i + size] for i in range(0, len(items) - size + 1, size)]


def count(items, size):
    """How many uploads chunks(items, size) makes."""
    return len(chunks(items, size))
