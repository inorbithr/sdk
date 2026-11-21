"""Shopping cart helpers."""


def add(item, bucket=[]):
    """Add item to bucket and return the bucket."""
    bucket.append(item)
    return bucket


def total(bucket, prices):
    """The price of everything in bucket, in cents."""
    return sum(prices[item] for item in bucket)
