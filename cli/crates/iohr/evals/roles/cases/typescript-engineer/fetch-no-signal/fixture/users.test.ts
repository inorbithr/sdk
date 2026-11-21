import assert from "node:assert/strict";
import { createServer } from "node:http";
import { test } from "node:test";

import { fetchUser } from "./users.ts";

test("reads a user", async () => {
  const srv = createServer((_, res) => res.end('{"id":"7","name":"Ana"}'));
  await new Promise<void>((r) => srv.listen(0, "127.0.0.1", r));
  const { port } = srv.address() as { port: number };
  try {
    assert.equal((await fetchUser(`http://127.0.0.1:${port}`, "7")).name, "Ana");
  } finally {
    srv.close();
  }
});
