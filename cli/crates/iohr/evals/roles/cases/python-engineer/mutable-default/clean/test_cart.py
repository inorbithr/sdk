import unittest

from cart import add, total


class CartTest(unittest.TestCase):
    def test_total(self):
        self.assertEqual(total(add("tea", ["milk"]), {"tea": 300, "milk": 120}), 420)

    def test_each_new_cart_starts_empty(self):
        add("tea")
        self.assertEqual(add("milk"), ["milk"])
