"""The passes-on-init test: boots the service on a free port and checks what a service owes."""

import io
import json
import os
import threading
import unittest
import urllib.error
import urllib.request

from @@MODULE@@.__main__ import load_config
from @@MODULE@@.obs import Logger
from @@MODULE@@.server import Service, make_server


class Booted(unittest.TestCase):
    def setUp(self):
        self.sink = io.StringIO()
        self.svc = Service(Logger(self.sink))
        self.server = make_server("127.0.0.1", 0, self.svc)
        self.base = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.svc.ready.set()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)

    def get(self, path, headers=None):
        req = urllib.request.Request(self.base + path, headers=headers or {})
        try:
            with urllib.request.urlopen(req, timeout=5) as r:
                return r.status, r.read().decode(), r.headers
        except urllib.error.HTTPError as e:
            return e.code, e.read().decode(), e.headers

    def records(self):
        return [json.loads(line) for line in self.sink.getvalue().splitlines()]


class TestService(Booted):
    def test_probes_answer(self):
        self.assertEqual(self.get("/healthz")[0], 200)
        self.assertEqual(self.get("/readyz")[0], 200)
        status, body, _ = self.get("/v1/hello?name=Ada")
        self.assertEqual((status, json.loads(body)["message"]), (200, "Hello, Ada"))
        self.assertEqual(self.get("/nope")[0], 404)

    def test_not_ready_while_shutting_down(self):
        self.svc.ready.clear()
        self.assertEqual(self.get("/readyz")[0], 503)
        self.assertEqual(self.get("/healthz")[0], 200)

    def test_one_span_with_the_route_and_the_trace_id_in_the_log(self):
        canary = "canary-value-must-not-be-logged"
        os.environ["SERVICE_CANARY"] = canary
        self.addCleanup(os.environ.pop, "SERVICE_CANARY")
        trace = "4bf92f3577b34da6a3ce929d0e0e4736"
        _, _, headers = self.get(
            "/v1/hello?name=secret-query-value",
            {"traceparent": f"00-{trace}-00f067aa0ba902b7-01", "Authorization": "Bearer secret-header-value"},
        )
        self.assertTrue(headers["traceparent"].startswith(f"00-{trace}-"))
        spans = [r for r in self.records() if r["msg"] == "span"]
        requests = [r for r in self.records() if r["msg"] == "request"]
        self.assertEqual((len(spans), len(requests)), (1, 1))
        self.assertEqual(spans[0]["route"], "/v1/hello")
        self.assertEqual(spans[0]["trace_id"], trace)
        self.assertEqual(spans[0]["parent_span_id"], "00f067aa0ba902b7")
        self.assertEqual(requests[0]["trace_id"], trace)
        text = self.sink.getvalue()
        for secret in (canary, "secret-query-value", "secret-header-value"):
            self.assertNotIn(secret, text)
        for value in os.environ.values():
            if len(value) >= 12:
                self.assertNotIn(value, text)

    def test_an_unknown_path_is_never_logged_and_metrics_count_by_route(self):
        self.get("/private/user-1234")
        self.get("/healthz")
        self.assertNotIn("user-1234", self.sink.getvalue())
        metrics = self.get("/metrics")[1]
        self.assertIn('route="unmatched",status="404"', metrics)
        self.assertIn('route="/healthz",status="200"', metrics)


class TestConfig(unittest.TestCase):
    def test_defaults_and_a_bad_port(self):
        self.assertEqual(load_config({}), ("127.0.0.1", 8080))
        with self.assertRaises(ValueError):
            load_config({"PORT": "99999"})


if __name__ == "__main__":
    unittest.main()
