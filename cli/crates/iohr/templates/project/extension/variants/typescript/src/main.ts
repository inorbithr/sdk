// @@ID@@: the demo project's private macOS service.
//
// Every minute it writes $STATE/heartbeat.json (the health InOrbit reads) and asks iohr's
// token channel for a token with the one scope its manifest declares. It also serves a tiny
// panel on 127.0.0.1 that InOrbit Desktop can show as a pane: the page reads /api/heartbeat.
//
// Run it with `iohr ext dev` (the real token channel, $STATE and $CACHE under
// ~/.inorbit/ext-dev/@@NAME@@/), or once with IOHR_EXT_ONCE=1 as the self-test does.
// Node runs the TypeScript as it is (22.18 or newer), so there is nothing to build.

import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import type { AddressInfo } from "node:net";

import { Refused, token } from "./channel.ts";

export const ID = "@@ID@@";
export const VERSION = "0.1.0";
const SCOPES = ["identity:read"];
const PANEL = path.join(import.meta.dirname, "..", "panel", "index.html");

type Heartbeat = Record<string, string | number | boolean>;

function writeJson(state: string, name: string, value: object): void {
  const tmp = path.join(state, `.${name}.tmp`);
  fs.writeFileSync(tmp, JSON.stringify(value));
  fs.renameSync(tmp, path.join(state, name));
}

async function heartbeat(state: string, beats: number): Promise<Heartbeat> {
  let tok = "granted";
  try {
    await token(SCOPES); // kept in memory only, never printed
  } catch (e) {
    if (!(e instanceof Refused)) throw e;
    console.error(`token refused: ${e.message}`);
    tok = "refused";
  }
  const at = new Date().toISOString().replace(/\.\d{3}Z$/, "Z");
  const beat: Heartbeat = {
    id: ID,
    version: VERSION,
    state: "ok",
    at,
    beats,
    token: tok,
    dev: process.env.IOHR_EXT_DEV === "1",
  };
  writeJson(state, "heartbeat.json", beat);
  console.log(`heartbeat ${beats} at ${at} (token ${tok})`);
  return beat;
}

/** Listens on 127.0.0.1 only; the port is PANEL_PORT or one the system picks, and it is
 * written to $STATE/panel.json so the app knows where the panel is. */
function serve(state: string, latest: () => Heartbeat): Promise<http.Server> {
  const srv = http.createServer((req, res) => {
    if (req.url === "/api/heartbeat") {
      res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify(latest()));
    } else if (req.url === "/" || req.url === "/index.html") {
      res
        .writeHead(200, { "content-type": "text/html; charset=utf-8" })
        .end(fs.readFileSync(PANEL));
    } else {
      res.writeHead(404, { "content-type": "text/plain" }).end("not found");
    }
  });
  return new Promise((resolve) => {
    srv.listen(Number(process.env.PANEL_PORT ?? "0"), "127.0.0.1", () => {
      const port = (srv.address() as AddressInfo).port;
      writeJson(state, "panel.json", { id: ID, url: `http://127.0.0.1:${port}/` });
      console.log(`panel at http://127.0.0.1:${port}/`);
      resolve(srv);
    });
  });
}

async function main(): Promise<number> {
  const state = process.env.STATE;
  if (!state || !process.env.CACHE) {
    console.error("STATE and CACHE are not set: run this with iohr ext dev");
    return 2;
  }
  const every = Number(process.env.HEARTBEAT_EVERY ?? "60");
  const once = Boolean(process.env.IOHR_EXT_ONCE);
  console.log(`${ID} ${VERSION} starting; state in ${state}`);
  let latest: Heartbeat = {};
  if (!once) await serve(state, () => latest);
  process.on("SIGTERM", () => process.exit(0));
  process.on("SIGINT", () => {
    console.log("stopping");
    process.exit(0);
  });
  for (let beats = 1; ; beats++) {
    latest = await heartbeat(state, beats);
    if (once) return 0;
    await new Promise((r) => setTimeout(r, every * 1000));
  }
}

process.exitCode = await main();
