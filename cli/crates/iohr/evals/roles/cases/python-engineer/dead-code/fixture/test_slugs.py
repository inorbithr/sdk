import unittest

from slugs import slugify


class SlugTest(unittest.TestCase):
    def test_slugify(self):
        self.assertEqual(slugify("Hello, World 2"), "hello-world-2")
