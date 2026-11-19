import http.server
import threading
import unittest

from partner import stock


class Answer(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b'{"count": 4}')

    def log_message(self, *args):
        pass


class PartnerTest(unittest.TestCase):
    def test_stock(self):
        srv = http.server.HTTPServer(("127.0.0.1", 0), Answer)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        try:
            self.assertEqual(stock(f"http://127.0.0.1:{srv.server_port}", "a1"), 4)
        finally:
            srv.shutdown()
