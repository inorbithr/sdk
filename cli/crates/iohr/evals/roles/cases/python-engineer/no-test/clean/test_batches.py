import unittest

from batches import chunks


class ChunksTest(unittest.TestCase):
    def test_keeps_a_short_last_batch(self):
        self.assertEqual(chunks([1, 2, 3, 4, 5], 2), [[1, 2], [3, 4], [5]])

    def test_empty(self):
        self.assertEqual(chunks([], 3), [])
