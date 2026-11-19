import assert from "node:assert/strict";
import { createServer } from "node:http";
import { test } from "node:test";

import { fetchUser } from "./users.ts";

test("gives up on a silent service", { timeout: 30_000 }, async () => {
  const srv = createServer(() => {}); // takes the request, never answers
  await new Promise<void>((r) => srv.listen(0, "127.0.0.1", r));
  const { port } = srv.address() as { port: number };
  let timer: NodeJS.Timeout | undefined;
  const late = new Promise<string>((r) => { timer = setTimeout(() => r("still waiting"), 15_000); });
  const call = fetchUser(`http://127.0.0.1:${port}`, "7").then(() => "answered", () => "gave up");
  const outcome = await Promise.race([call, late]);
  clearTimeout(timer);
  srv.closeAllConnections();
  srv.close();
  assert.equal(outcome, "gave up", "fetchUser still waits after 15s: fetch has no signal");
});
