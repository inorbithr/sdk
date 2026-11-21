import logging
import unittest

import reset


class EvalCheck(unittest.TestCase):
    def test_the_token_is_not_logged(self):
        with self.assertLogs("accounts", level=logging.DEBUG) as logs:
            logging.getLogger("accounts").debug("start")
            token = reset.start_reset("ana@example.com", [])
        for line in logs.output:
            self.assertNotIn(token, line, "the reset token is in the log")
