import unittest

from cart import add, total


class CartTest(unittest.TestCase):
    def test_total(self):
        self.assertEqual(total(add("tea", ["milk"]), {"tea": 300, "milk": 120}), 420)
