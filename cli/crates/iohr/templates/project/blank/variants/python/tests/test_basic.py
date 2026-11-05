import unittest

from @@MODULE@@ import hello


class Basic(unittest.TestCase):
    def test_hello(self):
        self.assertEqual(hello(), "hello from @@NAME@@")


if __name__ == "__main__":
    unittest.main()
