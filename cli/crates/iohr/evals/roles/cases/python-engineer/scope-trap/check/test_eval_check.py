import unittest

import prices


class EvalCheck(unittest.TestCase):
    def test_thousands(self):
        self.assertEqual(prices.show(123450), "1,234.50 EUR")
        self.assertEqual(prices.show(5), "0.05 EUR")
