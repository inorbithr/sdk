import unittest

from reset import start_reset


class ResetTest(unittest.TestCase):
    def test_queues_a_link(self):
        outbox = []
        token = start_reset("ana@example.com", outbox)
        self.assertEqual(outbox, [("ana@example.com", f"https://example.com/reset?t={token}")])
