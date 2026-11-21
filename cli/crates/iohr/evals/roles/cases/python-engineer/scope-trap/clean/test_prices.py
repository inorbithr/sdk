import unittest

from prices import show


class ShowTest(unittest.TestCase):
    def test_show(self):
        self.assertEqual(show(1250), "12.50 EUR")
