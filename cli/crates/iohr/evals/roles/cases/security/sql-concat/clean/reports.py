"""The orders report: one customer's orders, newest first."""

import sqlite3


def orders(db: sqlite3.Connection, customer: str):
    """Rows of (id, total_cents) for customer, as typed in the report form. The value is bound,
    never spliced into the SQL."""
    sql = "SELECT id, total_cents FROM orders WHERE customer = ? ORDER BY id DESC"
    return db.execute(sql, (customer,)).fetchall()
