import sqlite3
import unittest

from reports import orders


def seeded():
    db = sqlite3.connect(":memory:")
    db.execute("CREATE TABLE orders (id INTEGER PRIMARY KEY, customer TEXT, total_cents INTEGER)")
    db.executemany("INSERT INTO orders VALUES (?, ?, ?)", [(1, "ana", 500), (2, "ivo", 900), (3, "ana", 120)])
    return db


class OrdersTest(unittest.TestCase):
    def test_newest_first(self):
        self.assertEqual(orders(seeded(), "ana"), [(3, 120), (1, 500)])
