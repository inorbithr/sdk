"""The self-test `iohr ext dev --test` runs, with the same token channel, STATE and CACHE.

It starts a fake coordinator on port 0 (no mock of ours: a stand-in for the HTTP API only),
runs the extension against it and checks the AI label, the streamed reply, that the
question is the message body only, and that nothing was written into the folder."""

import http.server
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import unittest

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SEEN = []
ASKED = []  # the scopes each token request on the fake channel carried


def serve_channel(path):
    """A fake IOHR_EXT_TOKEN_SOCKET: answers `POST /token` as iohr does, and records the scopes."""
    srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    srv.bind(path)
    srv.listen(4)

    def loop():
        while True:
            try:
                conn, _ = srv.accept()
            except OSError:
                return
            with conn:
                raw = b""
                while b"\r\n\r\n" not in raw:
                    raw += conn.recv(4096)
                head, _, body = raw.partition(b"\r\n\r\n")
                want = int(head.lower().split(b"content-length:")[1].split(b"\r\n")[0])
                while len(body) < want:
                    body += conn.recv(4096)
                scopes = json.loads(body)["scopes"]
                ASKED.append(scopes)
                if "tokens:write" in scopes:
                    out, status = {"error": "forbidden", "message": "scope not declared"}, b"403 Forbidden"
                else:
                    out, status = {"access_token": "channel-token", "expires_at": "2099-01-01T00:00:00Z"}, b"200 OK"
                payload = json.dumps(out).encode()
                conn.sendall(b"HTTP/1.1 " + status + b"\r\ncontent-type: application/json\r\ncontent-length: "
                             + str(len(payload)).encode() + b"\r\nconnection: close\r\n\r\n" + payload)

    threading.Thread(target=loop, daemon=True).start()
    return srv


class Fake(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def _send(self, obj, sse=False):
        self.send_response(200)
        self.send_header("content-type", "text/event-stream" if sse else "application/json")
        self.end_headers()
        if sse:
            for o in obj:
                self.wfile.write(b"data: " + json.dumps({"result": o}).encode() + b"\n\n")
        else:
            self.wfile.write(json.dumps(obj).encode())

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))) or b"{}")
        SEEN.append((self.path, body, self.headers.get("authorization")))
        if self.path == "/mcp":
            if "EMPTY" in json.dumps(body):
                self._send({"jsonrpc": "2.0", "id": 1, "result": {"structuredContent": {"text": ""}}})
            else:
                chunks = [{"text": "mcp answer"}, {"text": "SECRET-THOUGHT", "reasoning": True}]
                self._send({"jsonrpc": "2.0", "id": 1, "result": {"structuredContent": {"chunks": chunks}}})
        elif self.path == "/v1/llm/generate/events":
            self.send_response(200)
            self.send_header("content-type", "text/event-stream")
            self.end_headers()
            for ev, o in [("chunk", {"text": "SECRET-THOUGHT", "reasoning": True}), ("chunk", {"text": "streamed "}),
                          ("chunk", {"text": "answer"})]:
                self.wfile.write(f"event: {ev}\ndata: {json.dumps({'result': o})}\n\n".encode())
        elif self.path.endswith("/threads"):
            self._send({"thread": {"threadId": "t1"}})
        elif self.path.endswith("/messages"):
            self._send({"message": {"messageId": "m1"}})
        else:
            self._send({})

    def do_GET(self):
        self._send([
            {"message": {"messageId": "m1", "kind": "say", "body": "QUESTION"}},
            {"message": {"messageId": "m2", "kind": "reply", "body": "the answer"}},
            {"turn": {"state": "replied"}},
        ], sse=True)


class AskTest(unittest.TestCase):
    def run_ask(self, extra_env):
        srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fake)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        tmp = tempfile.mkdtemp(prefix="ask")
        chan = serve_channel(os.path.join(tmp, "t.sock"))
        env = dict(os.environ, ASK_QUESTION="QUESTION", PYTHONDONTWRITEBYTECODE="1",
                   PYTHONPATH=os.path.join(HERE, "src"), IOHR_EXT_API=f"http://127.0.0.1:{srv.server_port}",
                   IOHR_EXT_TOKEN_SOCKET=os.path.join(tmp, "t.sock"))
        env.pop("ASK_BACKEND", None)
        env.update(extra_env)
        before = sorted(os.listdir(HERE))
        try:
            r = subprocess.run([sys.executable, "-m", "ask"], env=env, cwd=HERE, capture_output=True, text=True)
        finally:
            srv.shutdown()
            srv.server_close()
            chan.close()
            shutil.rmtree(tmp, ignore_errors=True)
        self.assertEqual(sorted(os.listdir(HERE)), before, "the extension wrote into its folder")
        return r

    def test_label_stream_and_question_only_in_body(self):
        SEEN.clear()
        ASKED.clear()
        r = self.run_ask({"ASK_BACKEND": "say"})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("AI answer", r.stdout)
        self.assertIn("the answer", r.stdout)
        self.assertNotIn("QUESTION", r.stdout)
        self.assertNotIn("QUESTION", r.stderr)
        self.assertEqual(ASKED, [["coordinator:read", "coordinator:write"]])
        self.assertTrue(all(a == "Bearer channel-token" for _, _, a in SEEN))
        posts = {p: b for p, b, _ in SEEN}
        self.assertNotIn("QUESTION", json.dumps(posts["/v1/coordinator/threads"]))
        self.assertEqual(posts["/v1/coordinator/threads/t1/messages"], {"body": "QUESTION"})

    def test_mcp_backend_prints_the_text_not_the_reasoning(self):
        SEEN.clear()
        ASKED.clear()
        r = self.run_ask({})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("generated by an AI model", r.stdout)
        self.assertIn("mcp answer", r.stdout)
        self.assertNotIn("SECRET-THOUGHT", r.stdout)
        # The question goes to the API only, never to the terminal.
        self.assertNotIn("QUESTION", r.stdout)
        self.assertNotIn("QUESTION", r.stderr)
        self.assertEqual(SEEN[0][0], "/mcp")
        self.assertEqual(SEEN[0][2], "Bearer channel-token")
        self.assertEqual(ASKED, [["mcp:generate", "mcp:read"]])

    def test_generate_backend_streams_the_text_not_the_reasoning(self):
        SEEN.clear()
        r = self.run_ask({"ASK_BACKEND": "generate"})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("generated by an AI model", r.stdout)
        self.assertIn("streamed answer", r.stdout)
        self.assertNotIn("SECRET-THOUGHT", r.stdout)
        self.assertNotIn("QUESTION", r.stdout)
        self.assertNotIn("QUESTION", r.stderr)
        self.assertEqual(SEEN[0][0], "/v1/llm/generate/events")

    def test_empty_answer_fails_without_the_label(self):
        r = self.run_ask({"ASK_QUESTION": "EMPTY"})
        self.assertNotEqual(r.returncode, 0)
        self.assertNotIn("AI answer", r.stdout)
        self.assertNotIn("generated by an AI model", r.stdout)
        self.assertNotIn("EMPTY", r.stdout + r.stderr)

    def test_no_channel_means_no_token(self):
        SEEN.clear()
        r = self.run_ask({"IOHR_EXT_TOKEN_SOCKET": ""})
        self.assertEqual(r.returncode, 1)
        self.assertIn("token refused", r.stderr)
        self.assertEqual(SEEN, [], "no call may be made without a token from the channel")

    def test_an_undeclared_scope_is_refused(self):
        sys.path.insert(0, os.path.join(HERE, "src"))
        from importlib import import_module

        channel = import_module("ask.channel")
        with self.assertRaises(channel.Refused):
            channel.token(["tokens:write"])

    def test_no_question_is_usage_error(self):
        r = self.run_ask({"ASK_QUESTION": ""})
        self.assertEqual(r.returncode, 2)


if __name__ == "__main__":
    unittest.main()
