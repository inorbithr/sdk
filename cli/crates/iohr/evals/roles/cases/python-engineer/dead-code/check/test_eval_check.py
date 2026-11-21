import unittest

import slugs


class EvalCheck(unittest.TestCase):
    def test_unique_slug(self):
        self.assertEqual(slugs.unique_slug("Hi there", set()), "hi-there")
        self.assertEqual(slugs.unique_slug("Hi there", {"hi-there", "hi-there-2"}), "hi-there-3")
