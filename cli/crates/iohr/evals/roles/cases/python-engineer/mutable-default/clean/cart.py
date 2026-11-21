"""Shopping cart helpers."""


def add(item, bucket=None):
    """Add item to bucket (a new one when none is given) and return the bucket."""
    if bucket is None:
        bucket = []
    bucket.append(item)
    return bucket


def total(bucket, prices):
    """The price of everything in bucket, in cents."""
    return sum(prices[item] for item in bucket)
