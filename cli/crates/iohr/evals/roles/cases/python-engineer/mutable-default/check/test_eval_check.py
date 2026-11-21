import unittest

import cart


class EvalCheck(unittest.TestCase):
    def test_new_carts_do_not_share_items(self):
        first = cart.add("tea")
        second = cart.add("milk")
        self.assertEqual(second, ["milk"], "two new carts share one default list")
        self.assertEqual(first, ["tea"])
