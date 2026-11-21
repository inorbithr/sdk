import unittest

import batches


class EvalCheck(unittest.TestCase):
    def test_no_item_is_dropped(self):
        self.assertEqual(batches.chunks(list(range(7)), 3), [[0, 1, 2], [3, 4, 5], [6]])
        self.assertEqual(batches.chunks([1], 5), [[1]])
        self.assertEqual(batches.chunks([], 2), [])
