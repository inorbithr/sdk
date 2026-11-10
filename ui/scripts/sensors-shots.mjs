#!/usr/bin/env node
// Headless screenshots of the Sensors page (RFC 0217): `node scripts/sensors-shots.mjs <outdir>` after `pnpm run build`.
// There is no signed-in `iohr ui serve` in a headless run, so this serves `dist` itself, puts a start token in the page
// and answers `/api/session` and `/api/proxy/v1/...` with FIXTURE data (every name and value below is made up, and the
// output folder says so). The signal stream is a real server-sent stream that emits a reading a second, so "live" is
// exercised, not painted. Set PLAYWRIGHT_DIR to a folder with `playwright` installed when it is not a dependency here.
// It also tries every exit: Escape and the back button out of a planet, Escape, X, Done and a click outside out of "Your
// sensors", and a reload keeps the view and the pins.
import { readFileSync, mkdirSync, existsSync } from "node:fs";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const dist = join(root, "dist");
if (!existsSync(join(dist, "index.html"))) throw new Error("run `pnpm run build` first");
const out = resolve(process.argv[2] || "sensors-shots-fixture");
mkdirSync(out, { recursive: true });
const { chromium } = process.env.PLAYWRIGHT_DIR ? createRequire(`${process.env.PLAYWRIGHT_DIR}/`)("playwright") : await import("playwright");

const TOKEN = "a".repeat(64);
const ME = "fixture-person";
const now = () => Date.now();
const ago = (s) => new Date(now() - s * 1000).toISOString();
const S = (id, source, name, over = {}) => ({ sensor_id: id, name, source, kind: "http", state: "active", health: "ok", interval_seconds: 30, rate_per_minute: 2, last_seen_at: ago(10), owner: "", builtin: true, schema_version: 1, ...over });
const sensors = [
  S("m1", "monitors", "Fixture API latency", { kind: "latency", unit: "ms", rate_per_minute: 12 }),
  S("m2", "monitors", "Fixture web uptime", { health: "warning", rate_per_minute: 6 }),
  S("m3", "monitors", "Fixture queue depth", { health: "down", kind: "jobs" }),
  S("c1", "coordinator", "Fixture running jobs", { kind: "jobs", rate_per_minute: 20 }),
  S("a1", "asks", "Fixture open asks", { kind: "jobs", rate_per_minute: 1 }),
  S("i1", "incidents", "Fixture open incidents", { kind: "incidents", rate_per_minute: 1 }),
  S("p1", "agent", "Fixture build agent", { builtin: false, owner: ME, producer_kind: "agent", producer_ref: "build-1", rate_per_minute: 30 }),
  S("p2", "agent", "Fixture notes agent", { builtin: false, owner: ME, state: "paused", producer_kind: "agent", producer_ref: "notes-1", last_seen_at: ago(4000) }),
  S("p3", "agent", "Fixture someone else's agent", { builtin: false, owner: "fixture-other", producer_kind: "agent", producer_ref: "x" }),
  S("s1", "server", "Fixture server cpu", { kind: "cpu", unit: "%", builtin: false, owner: "fixture-other", rate_per_minute: 20, health: "ok", last_seen_at: ago(5000) }),
];
const wave = (id, n = 40) => Array.from({ length: n }, (_, i) => ({ sensor_id: id, produced_at: ago((n - i) * 30), received_at: ago((n - i) * 30), value: 50 + 30 * Math.sin(i / 4 + id.charCodeAt(0)), state: "ok" }));
const groups = () => {
  const by = new Map();
  for (const s of sensors) by.set(s.source, [...(by.get(s.source) ?? []), s]);
  return [...by].map(([source, ss]) => ({ source, sensors: ss }));
};
const jobs = [
  { job_id: "j1", thread_id: "t1", kind: "build", brief: "Fixture: my own running job", state: "running", stage: "testing", created_by: ME, updated_at: ago(60) },
  { job_id: "j2", thread_id: "t2", kind: "build", brief: "Fixture: someone else's secret job", state: "running", created_by: "fixture-other", updated_at: ago(90) },
];
const sent = [];

const index = readFileSync(join(dist, "index.html"), "utf8").replace("<head>", `<head><meta name="iohr-token" content="${TOKEN}">`);
const mime = { ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".json": "application/json" };
const server = createServer((req, res) => {
  const u = new URL(req.url, "http://x");
  const json = (o, code = 200) => (res.writeHead(code, { "content-type": "application/json" }), res.end(JSON.stringify(o)));
  if (req.headers["x-iohr-token"] === undefined && (u.pathname.startsWith("/api/proxy") || u.pathname === "/api/session")) return json({ error: "no token" }, 401);
  if (u.pathname === "/api/session") return json({ signed_in: true, profile: "fixture", kind: "person", account: "fixture-account", subject: ME, scopes: [], login: "idle" });
  if (u.pathname === "/api/extensions") return json({ extensions: [] });
  const p = u.pathname.replace("/api/proxy", "");
  if (u.pathname.startsWith("/api/proxy")) {
    if (p === "/v1/atlas/signals/events") {
      res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-store" });
      let n = 0;
      const t = setInterval(() => {
        const id = ["m1", "p1", "c1", "m2"][n++ % 4];
        res.write(`id: ${n}\nevent: reading\ndata: ${JSON.stringify({ sse_id: String(n), type: "reading", reading: { sensor_id: id, produced_at: ago(0), received_at: ago(0), value: 40 + (n % 7) * 5, state: id === "m2" ? "warning" : "ok" } })}\n\n`);
      }, 1000);
      req.on("close", () => clearInterval(t));
      return;
    }
    if (p === "/v1/coordinator/picture/events") return (res.writeHead(200, { "content-type": "text/event-stream" }), res.write(": open\n\n"));
    if (p === "/v1/atlas/sensors") return json({ groups: groups() });
    let m = /^\/v1\/atlas\/sensors\/([^/]+)\/readings$/.exec(p);
    if (m) return json({ readings: wave(m[1]) });
    m = /^\/v1\/atlas\/sensors\/([^/]+)\/(pause|resume|revoke)$/.exec(p);
    if (m && req.method === "POST") {
      const s = sensors.find((x) => x.sensor_id === m[1]);
      if (!s || s.owner !== ME) return json({ error: "not yours" }, 403);
      s.state = m[2] === "pause" ? "paused" : m[2] === "resume" ? "active" : "revoked";
      sent.push(`${m[2]} ${m[1]}`);
      return json({});
    }
    m = /^\/v1\/atlas\/sensors\/([^/]+)\/sent$/.exec(p);
    if (m) return req.method === "DELETE" ? json({}) : json({ sensor_id: m[1], readings: "1240", rollups: "12", first_at: "2026-11-01T00:00:00Z" });
    if (p === "/v1/coordinator/jobs") return json({ jobs });
    if (p === "/v1/coordinator/threads" || p === "/v1/coordinator/asks") return json({ threads: [], asks: [] });
    if (/\/view-requests$/.test(p) && req.method === "POST") return json({ view_request_id: "vwr_fixture" });
    return json({ error: "unknown" }, 404);
  }
  const file = join(dist, u.pathname);
  if (u.pathname !== "/" && file.startsWith(dist) && existsSync(file) && !file.endsWith("/")) {
    res.writeHead(200, { "content-type": mime[file.slice(file.lastIndexOf("."))] ?? "application/octet-stream" });
    return res.end(readFileSync(file));
  }
  res.writeHead(200, { "content-type": "text/html" });
  res.end(index);
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}`;

let failures = 0;
const check = (ok, what) => (console.log(ok ? "ok  " : "FAIL", what), ok || failures++);
let browser;
try {
  browser = await chromium.launch().catch(() => chromium.launch({ channel: "chrome" }));
  const open = async ({ scheme = "light", width = 1280, height = 860, path = "/sensors", reduced = "no-preference" } = {}) => {
    const ctx = await browser.newContext({ viewport: { width, height }, colorScheme: scheme, reducedMotion: reduced });
    const tab = await ctx.newPage();
    const errors = [];
    tab.on("pageerror", (e) => errors.push(String(e)));
    await tab.goto(base + path);
    await tab.getByRole("group", { name: "View" }).waitFor({ timeout: 15000 });
    await tab.getByTestId("sensors-conn").getByText("Live").waitFor({ timeout: 15000 });
    await tab.waitForTimeout(1500);
    return { ctx, tab, errors };
  };
  for (const scheme of ["light", "dark"]) {
    for (const [name, width, height] of [["desktop", 1280, 860], ["phone", 390, 844]]) {
      const { ctx, tab, errors } = await open({ scheme, width, height });
      await tab.screenshot({ path: join(out, `orbit-${scheme}-${name}.png`) });
      await tab.getByRole("button", { name: "Control room" }).click();
      await tab.waitForTimeout(1200);
      await tab.screenshot({ path: join(out, `control-${scheme}-${name}.png`) });
      check(errors.length === 0, `no page errors (${scheme} ${name})`);
      await ctx.close();
    }
  }

  // A planet: zoom, panel, sessions, exits.
  {
    const { ctx, tab } = await open({ scheme: "light" });
    const planet = tab.locator("[data-planet]").first();
    check((await planet.count()) > 0, "planets are in the page");
    const coordinator = tab.getByRole("button", { name: /^Coordinator/ });
    await coordinator.first().click();
    await tab.waitForTimeout(1000);
    check(await tab.getByText("Recent sessions").isVisible(), "the planet panel shows Recent sessions");
    check(await tab.getByText("Fixture: my own running job").isVisible(), "my own session is shown in full");
    check(!(await tab.getByText("someone else's secret job").count()), "someone else's session is redacted");
    check(await tab.getByRole("button", { name: "Ask to view" }).isVisible(), "Ask to view is offered for someone else's session");
    await tab.getByRole("button", { name: "Ask to view" }).click();
    await tab.getByText("Asked. Its owner sees your request").waitFor({ timeout: 5000 });
    check(true, "Ask to view posts and says so");
    await tab.screenshot({ path: join(out, "planet-light-desktop.png") });
    await tab.keyboard.press("Escape");
    await tab.waitForTimeout(900);
    check((await tab.getByText("Recent sessions").count()) === 0, "exit: Escape leaves the planet");
    await coordinator.first().click();
    await tab.waitForTimeout(900);
    await tab.getByRole("button", { name: /Back to all sources/ }).click();
    await tab.waitForTimeout(900);
    check((await tab.getByText("Recent sessions").count()) === 0, "exit: the back button leaves the planet");
    await ctx.close();
  }
  {
    const { ctx, tab } = await open({ scheme: "dark" });
    await tab.getByRole("button", { name: /^Monitors/ }).first().click();
    await tab.waitForTimeout(1000);
    await tab.screenshot({ path: join(out, "planet-dark-desktop.png") });
    await ctx.close();
  }

  // Your sensors: the consent sheet, per person, from the service.
  for (const scheme of ["light", "dark"]) {
    const { ctx, tab } = await open({ scheme });
    await tab.getByTestId("your-sensors-open").click();
    const dlg = tab.getByRole("dialog", { name: "Your sensors" });
    await dlg.waitFor();
    const rows = await dlg.getByTestId("your-sensor").count();
    check(rows === 2, `${scheme}: only my 2 sensors are listed (got ${rows})`);
    check(!(await dlg.getByText("someone else's agent").count()), `${scheme}: another person's sensor is not listed`);
    if (scheme === "light") {
      const sw = dlg.getByRole("switch", { name: "Send Fixture build agent" });
      check((await sw.getAttribute("aria-checked")) === "true", "my active sensor is on");
      await sw.click();
      await tab.waitForTimeout(800);
      check((await sw.getAttribute("aria-checked")) === "false", "turning it off pauses it (the service answered)");
      await sw.click();
      await tab.waitForTimeout(800);
      await dlg.getByRole("button", { name: "What was sent" }).first().click();
      await dlg.getByText("1240 readings and 12 rollups since 2026-11-01.").waitFor({ timeout: 5000 });
      check(true, "what was sent is read from the service");
    }
    await tab.screenshot({ path: join(out, `consent-${scheme}-desktop.png`) });
    if (scheme === "light") {
      await dlg.getByRole("button", { name: "Revoke", exact: true }).first().click();
      check(await dlg.getByRole("button", { name: "Revoke for good" }).isVisible(), "revoke asks again before it acts");
      await dlg.getByRole("button", { name: "Keep it" }).first().click();
      // exits, each tried
      await tab.keyboard.press("Escape");
      check((await dlg.count()) === 0, "exit: Escape closes the sheet");
      await tab.getByTestId("your-sensors-open").click();
      await tab.getByRole("button", { name: "Close" }).click();
      check((await dlg.count()) === 0, "exit: X closes the sheet");
      await tab.getByTestId("your-sensors-open").click();
      await dlg.getByRole("button", { name: "Done" }).click();
      check((await dlg.count()) === 0, "exit: Done closes the sheet");
      await tab.getByTestId("your-sensors-open").click();
      await tab.mouse.click(5, 5);
      check((await dlg.count()) === 0, "exit: a click outside closes the sheet");
    }
    await ctx.close();
  }
  {
    const { ctx, tab } = await open({ scheme: "light", width: 390, height: 844 });
    await tab.getByTestId("your-sensors-open").click();
    await tab.getByRole("dialog").waitFor();
    await tab.screenshot({ path: join(out, "consent-light-phone.png") });
    await ctx.close();
  }

  // Reduced motion, and the view and pins survive a reload.
  {
    const { ctx, tab } = await open({ reduced: "reduce" });
    await tab.screenshot({ path: join(out, "orbit-reduced-motion.png") });
    await tab.getByRole("button", { name: "Control room" }).click();
    await tab.getByRole("button", { name: "Pin Fixture web uptime" }).click();
    await tab.reload();
    await tab.getByRole("group", { name: "View" }).waitFor();
    check((await tab.getByRole("button", { name: "Control room" }).getAttribute("aria-pressed")) === "true", "relaunch: the view is kept");
    check((await tab.getByRole("button", { name: "Unpin Fixture web uptime" }).count()) === 1, "relaunch: the pin is kept");
    check(new URL(tab.url()).searchParams.get("view") === "control", "the view is in the address");
    await ctx.close();
  }
} finally {
  await browser?.close();
  server.close();
}
console.log(failures ? `${failures} FAILED` : "all ok", `-> ${out}`);
process.exit(failures ? 1 : 0);
