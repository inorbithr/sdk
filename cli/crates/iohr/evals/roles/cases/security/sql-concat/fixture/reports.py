"""The orders report: one customer's orders, newest first."""

import sqlite3


def orders(db: sqlite3.Connection, customer: str):
    """Rows of (id, total_cents) for customer, as typed in the report form."""
    sql = "SELECT id, total_cents FROM orders WHERE customer = '" + customer + "' ORDER BY id DESC"
    return db.execute(sql).fetchall()
