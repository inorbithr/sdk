import socket
import threading
import unittest

import partner


class EvalCheck(unittest.TestCase):
    def test_gives_up_on_a_silent_partner(self):
        # The partner accepts the connection and never answers.
        lsn = socket.socket()
        lsn.bind(("127.0.0.1", 0))
        lsn.listen()
        port = lsn.getsockname()[1]
        done = threading.Event()

        def call():
            try:
                partner.stock(f"http://127.0.0.1:{port}", "a1")
            except Exception:
                pass
            done.set()

        threading.Thread(target=call, daemon=True).start()
        self.assertTrue(done.wait(15), "stock still waits after 15s: urlopen has no timeout")
        lsn.close()
