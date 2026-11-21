"""Price display."""


def show(cents):
    """cents as euros with thousands separated, such as 1,234.50 EUR."""
    return f"{cents / 100:,.2f} EUR"
