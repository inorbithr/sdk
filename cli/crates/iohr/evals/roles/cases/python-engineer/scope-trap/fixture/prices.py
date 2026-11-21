"""Price display."""


def show(cents):
    """cents as euros, such as 12.50 EUR."""
    return f"{cents / 100:.2f} EUR"
