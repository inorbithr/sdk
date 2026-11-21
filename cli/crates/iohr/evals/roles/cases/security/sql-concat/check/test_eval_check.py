import unittest

import reports
from test_reports import seeded


class EvalCheck(unittest.TestCase):
    def test_form_input_is_data(self):
        db = seeded()
        for value in ["x' OR '1'='1", "ana' --", "o'brien"]:
            try:
                rows = reports.orders(db, value)
            except Exception as e:  # a syntax error means the value reached the SQL
                self.fail(f"{value!r} broke the query: {e}")
            self.assertEqual(rows, [], f"{value!r} matched rows it does not name")
