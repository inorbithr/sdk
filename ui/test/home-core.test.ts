// The Home story (src/lib/home-core.ts): only what the API answered, never a made-up number or time.
import assert from "node:assert/strict";
import { test } from "node:test";
import { proxyPath } from "../src/lib/api.ts";
import {
  cleanTitle,
  dayOf,
  firstName,
  greeting,
  isFirstRun,
  lastCheck,
  ledeOf,
  type Monitor,
  servicesOf,
  storyFilter,
  storyOf,
  stripLine,
  stripOf,
  watched,
} from "../src/lib/home-core.ts";
import { homeReads, runsRead } from "../src/lib/home-data.ts";

const NOW = Date.parse("2026-11-10T12:00:00Z");
const ago = (s: number) => new Date(NOW - s * 1000).toISOString();

test("the greeting follows the hour and drops the name it does not have", () => {
  assert.equal(greeting("Nevio", 3), "Good evening, Nevio");
  assert.equal(greeting("Nevio", 9), "Good morning, Nevio");
  assert.equal(greeting("Nevio", 14), "Good afternoon, Nevio");
  assert.equal(greeting("Nevio", 21), "Good evening, Nevio");
  assert.equal(greeting("", 9), "Good morning");
});

test("the first name comes from the name, else the email, else nothing", () => {
  assert.equal(firstName({ name: "Nevio Vesic", email: "a@b.c" }), "Nevio");
  assert.equal(firstName({ email: "emma.k@x.io" }), "emma.k");
  assert.equal(firstName({}), "");
  assert.equal(firstName(null), "");
});

test("a health the API did not give is not known, never up", () => {
  const monitors: Monitor[] = [
    { monitor_id: "a", name: "auth", health: "failing", state: "active" },
    { monitor_id: "b", name: "site", health: "healthy", state: "active" },
    { monitor_id: "c", name: "old", health: "healthy", state: "paused" },
    { monitor_id: "d", name: "new", health: "", state: "active" },
  ];
  const s = servicesOf(monitors);
  assert.deepEqual(
    s.map((x) => [x.id, x.up, x.unknown, x.paused]),
    [
      ["a", false, false, false],
      ["b", true, false, false],
      ["c", true, false, true],
      ["d", false, true, false],
    ],
  );
  assert.deepEqual(
    watched(s).map((x) => x.id),
    ["a", "b", "d"],
  );
});

test("titles lose document numbers and ids", () => {
  assert.equal(cleanTitle("PRD 0029: The welcome"), "The welcome");
  assert.equal(cleanTitle("The welcome (RFC 0217.1)"), "The welcome");
  assert.equal(cleanTitle(""), "A decision");
});

const services = servicesOf([
  { monitor_id: "auth", name: "Sign-in", health: "failing", state: "active", last_run_at: ago(20) },
  { monitor_id: "site", name: "Website", health: "healthy", state: "active", last_run_at: ago(40) },
]);

test("the story is newest first, keeps open problems and waiting decisions however old, and drops old project news", () => {
  const events = storyOf({
    now: NOW,
    services,
    incidents: [
      { incident_id: "i1", title: "Sign-in fails", started_at: ago(3600), monitors: ["auth"] },
      { incident_id: "i2", title: "Old", started_at: ago(9000), resolved_at: ago(100) },
    ],
    reviews: [
      { review_id: "r1", document_id: "d1", requested_at: ago(86400 * 20), title: "RFC 0001: Old ask", kind: "rfc" },
      { review_id: "r2", document_id: "d2", decision: "approve", requested_at: ago(10) },
      {
        review_id: "r3",
        document_id: "d3",
        decision: "pending",
        requested_at: ago(86400 * 21),
        title: "ADR 0002: Old pick",
        kind: "adr",
      },
    ],
    spaces: [
      { space_id: "s1", name: "Platform", documents: 988, open: 237, updated_at: ago(7200) },
      { space_id: "s2", name: "Stale", documents: 4, open: 1, updated_at: ago(86400 * 30) },
      { space_id: "s3", name: "Empty", documents: 0, updated_at: ago(10) },
      { space_id: "s4", name: "No time", documents: 3, open: 0 },
    ],
  });
  assert.deepEqual(
    events.map((e) => e.id),
    ["inc:i1", "sp:s1", "rev:r1", "rev:r3"],
  );
  assert.equal(events[0]?.source, "check");
  assert.equal(events[1]?.claim, "Platform changed: 751 of 988 done.");
  assert.equal(events[2]?.claim, "A proposal waits for your yes or no: “Old ask”.");
});

test("a down service no incident names is a problem of its own; one an incident names is told once", () => {
  const alone = storyOf({ now: NOW, services, incidents: [], reviews: [], spaces: [] });
  assert.deepEqual(
    alone.map((e) => [e.id, e.claim, e.source]),
    [["svc:auth", "Sign-in is down.", "check"]],
  );
  const named = storyOf({
    now: NOW,
    services,
    incidents: [{ incident_id: "i1", title: "x", started_at: ago(5), monitors: ["auth"] }],
    reviews: [],
    spaces: [],
  });
  assert.deepEqual(
    named.map((e) => e.id),
    ["inc:i1"],
  );
});

test("a record with no time goes first and says so; its time is never made up", () => {
  const events = storyOf({
    now: NOW,
    services: [],
    incidents: [{ incident_id: "i1", title: "A" }],
    reviews: [],
    spaces: [{ space_id: "s1", name: "P", documents: 2, open: 1, updated_at: ago(60) }],
  });
  assert.deepEqual(
    events.map((e) => [e.id, e.timed]),
    [
      ["inc:i1", false],
      ["sp:s1", true],
    ],
  );
});

test("the filters keep one kind each", () => {
  const events = storyOf({
    now: NOW,
    services,
    incidents: [],
    reviews: [{ review_id: "r1", document_id: "d", requested_at: ago(5), title: "T" }],
    spaces: [{ space_id: "s1", name: "P", documents: 2, open: 1, updated_at: ago(60) }],
  });
  assert.equal(storyFilter(events, "all").length, 3);
  assert.deepEqual(
    storyFilter(events, "problems").map((e) => e.id),
    ["svc:auth"],
  );
  assert.deepEqual(
    storyFilter(events, "decisions").map((e) => e.id),
    ["rev:r1"],
  );
  assert.deepEqual(
    storyFilter(events, "projects").map((e) => e.id),
    ["sp:s1"],
  );
});

test("the lede says what is true: the problem, the decisions, the services that are up; a quiet day only when it is", () => {
  const events = storyOf({
    now: NOW,
    services,
    incidents: [],
    reviews: [{ review_id: "r1", document_id: "d", requested_at: ago(5), title: "T" }],
    spaces: [],
  });
  const lede = ledeOf(events, { services: 2, down: 1, unknown: 0 });
  assert.deepEqual(
    lede.map((l) => l.text),
    ["Sign-in is down.", "One decision waits for you.", "The other 1 service is up."],
  );
  assert.deepEqual(lede[0]?.ref, { event: "svc:auth" });
  assert.deepEqual(
    ledeOf([], { services: 3, down: 0, unknown: 0 }).map((l) => l.text),
    ["All 3 services are up."],
  );
  assert.deepEqual(
    ledeOf([], { services: 0, down: 0, unknown: 0 }).map((l) => l.text),
    ["A quiet day: nothing needs you."],
  );
  assert.equal(ledeOf([], { services: 3, down: 0, unknown: 1 })[0]?.text, "2 services are up, 1 not known.");
});

test("the newest check is said in seconds, minutes or hours, and nothing without a time", () => {
  assert.equal(lastCheck([ago(40), ago(20)], NOW), "20 seconds ago");
  assert.equal(lastCheck([ago(1)], NOW), "1 second ago");
  assert.equal(lastCheck([ago(300)], NOW), "5 minutes ago");
  assert.equal(lastCheck([ago(7200)], NOW), "2 hours ago");
  assert.equal(lastCheck([undefined, "bad"], NOW), "");
});

test("the run strip reads oldest first and says how many failed", () => {
  const runs = [
    { ok: true, at: ago(10) },
    { ok: false, at: ago(20), status_code: 503 },
    { ok: false, at: ago(30), status_code: 503 },
  ];
  const strip = stripOf(runs);
  assert.deepEqual(
    strip.map((r) => r.at),
    [ago(30), ago(20), ago(10)],
  );
  assert.equal(stripLine(strip), "2 of the last 3 checks failed.");
  assert.equal(
    stripLine(stripOf([{ ok: false, at: ago(1), status_code: 503 }])),
    "1 of the last 1 checks failed, the last answer was 503.",
  );
  assert.equal(stripLine([]), "No checks have run yet.");
});

test("days and the first run", () => {
  assert.equal(dayOf(NOW - 1000, NOW), "Today");
  assert.equal(dayOf(NOW - 86_400_000, NOW), "Yesterday");
  assert.equal(isFirstRun({ decisions: 0, services: 0, projects: 0 }), true);
  assert.equal(isFirstRun({ decisions: 0, services: 1, projects: 0 }), false);
});

test("Home's reads are plain GETs under /v1 the local server will forward, with the account named once", () => {
  const r = homeReads("org 1");
  assert.equal(r.reviews.query.status, "waiting");
  assert.equal(r.monitors.path, "/v1/accounts/orgs/org%201/monitors");
  for (const x of [...Object.values(r), runsRead("org 1", "mon/1")]) {
    assert.ok(proxyPath(x.path, x.query) !== null, x.path);
  }
  assert.equal(runsRead("o", "m/1").path, "/v1/accounts/orgs/o/monitors/m%2F1/runs");
});
