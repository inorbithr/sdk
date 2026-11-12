// The Home page's story, the pure part (RFC 0217, slice 3). The Mac app's Home ("Story", iohr-mac
// src/lib/normal-concepts.ts) tells the day as a few plain sentences and a timeline of events, newest first. This is
// the same idea for `iohr ui serve`, built only from what the API answered: a count is a count of what came back, a
// time is the record's own time, and a thing with no time says so. No React, no DOM, so node tests it.

export type Me = { name?: string; email?: string };
export type Space = {
  space_id: string;
  name: string;
  documents?: number;
  open?: number;
  awaiting_review?: number;
  updated_at?: string;
};
export type Review = {
  review_id: string;
  document_id?: string;
  decision?: string;
  requested_at?: string;
  title?: string;
  kind?: string;
  space_name?: string;
};
export type Incident = {
  incident_id: string;
  title?: string;
  status?: string;
  state?: string;
  started_at?: string;
  resolved_at?: string;
  acknowledged_at?: string;
  monitors?: string[];
};
export type Monitor = {
  monitor_id: string;
  name?: string;
  state?: string;
  health?: string;
  last_run_at?: string;
  check?: { target?: { url?: string; host?: string } } | null;
};
export type Run = { ok: boolean; at: string; status_code?: number | null };

export type Service = { id: string; name: string; paused: boolean; up: boolean; unknown: boolean; lastRun?: string };

const DOWN = /fail|down|unhealthy|critical/i;
const UNKNOWN = /^(|unknown)$/i;

/** The monitors as services: paused ones are not watched, a health the API did not give is "not known", never "up". */
export function servicesOf(monitors: Monitor[]): Service[] {
  return monitors.map((m) => {
    const health = (m.health ?? "").trim();
    const state = (m.state ?? "").trim();
    return {
      id: m.monitor_id,
      name: (m.name ?? "").trim() || m.check?.target?.url || m.check?.target?.host || "Service",
      paused: state === "paused",
      up: !DOWN.test(`${health} ${state}`) && !UNKNOWN.test(health),
      unknown: UNKNOWN.test(health) && !DOWN.test(state),
      lastRun: m.last_run_at,
    };
  });
}

/** The services that are being watched. */
export const watched = (s: Service[]): Service[] => s.filter((x) => !x.paused);

/** "Good morning, Emma": by the hour where the person is. */
export function greeting(name: string, hour: number): string {
  const part = hour < 5 ? "Good evening" : hour < 12 ? "Good morning" : hour < 18 ? "Good afternoon" : "Good evening";
  return name === "" ? part : `${part}, ${name}`;
}

/** The first name from what `/v1/accounts/me` answered; empty when it said nothing usable. */
export function firstName(me: Me | null): string {
  const n = (me?.name ?? "").trim() || (me?.email ?? "").split("@")[0]?.trim() || "";
  return n.split(/\s+/)[0] ?? "";
}

/** A title without document ids and numbers ("PRD 0029: ..." or "... (PRD 0029)"). */
export const cleanTitle = (title: string): string =>
  title
    .replace(/\bldoc_[0-9A-Z]+/g, "the document")
    .replace(/^\s*(?:PRD|RFC|ADR|study)\s*\d+(?:\.\d+)?\s*[:·,-]\s*/i, "")
    .replace(/\s*\((?:PRD|RFC|ADR|study)\s*\d+(?:\.\d+)?\)\s*$/i, "")
    .trim() || "A decision";

const kindWord = (kind?: string): string =>
  ({ prd: "A plan", rfc: "A proposal", adr: "A choice", study: "Findings" })[(kind ?? "").toLowerCase()] ??
  "A decision";

/** How far a space's documents are: done is what is not open. */
export function progressOf(s: Space): { done: number; total: number; state: "waiting" | "done" | "working" } {
  const total = s.documents ?? 0;
  const done = Math.max(0, total - (s.open ?? 0));
  const state = (s.awaiting_review ?? 0) > 0 ? "waiting" : total > 0 && done === total ? "done" : "working";
  return { done, total, state };
}

export type Kind = "problem" | "decision" | "project";
export type Event = {
  id: string;
  kind: Kind;
  at: number;
  timed: boolean;
  claim: string;
  /** "check" when a monitor saw it, "record" when it is what the record says. */
  source: "check" | "record";
  detail: string;
  monitor?: string;
  space?: string;
};
export type Filter = "all" | "problems" | "decisions" | "projects";
export const FILTERS: { id: Filter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "problems", label: "Problems" },
  { id: "decisions", label: "Decisions" },
  { id: "projects", label: "Projects" },
];

const at = (iso?: string): number => (iso ? Date.parse(iso) : Number.NaN);
const isOpen = (i: Incident) => !i.resolved_at && !/resolved|closed/i.test(`${i.status ?? ""} ${i.state ?? ""}`);

/**
 * The day as events, newest first. An open problem and a waiting decision stay however old they are; project news
 * older than `days` is left out. A thing with no time on its record goes first and says so, never a made-up time.
 */
export function storyOf(o: {
  incidents: Incident[];
  services: Service[];
  reviews: Review[];
  spaces: Space[];
  now?: number;
  days?: number;
}): Event[] {
  const now = o.now ?? Date.now();
  const since = now - (o.days ?? 7) * 86_400_000;
  const out: Event[] = [];
  const known = new Set(o.services.map((s) => s.id));
  const named = new Set<string>();
  for (const i of o.incidents.filter(isOpen)) {
    const t = at(i.started_at);
    const monitor = (i.monitors ?? []).find((m) => known.has(m));
    out.push({
      id: `inc:${i.incident_id}`,
      kind: "problem",
      at: Number.isNaN(t) ? now : t,
      timed: !Number.isNaN(t),
      claim: `${(i.title ?? "").trim() || "A service had a problem"}.`,
      source: monitor ? "check" : "record",
      detail: i.acknowledged_at ? "Someone is on it." : "Nobody has picked this up yet.",
      monitor,
    });
    for (const m of i.monitors ?? []) named.add(m);
  }
  for (const s of o.services) {
    if (s.paused || s.unknown || s.up || named.has(s.id)) continue;
    const t = at(s.lastRun);
    out.push({
      id: `svc:${s.id}`,
      kind: "problem",
      at: Number.isNaN(t) ? now : t,
      timed: !Number.isNaN(t),
      claim: `${s.name} is down.`,
      source: "check",
      detail: "Nobody has picked this up yet.",
      monitor: s.id,
    });
  }
  for (const r of o.reviews) {
    // Still open: the proto says "pending" (the Mac app's rows carry it empty); an answered one ("approve", "changes") is not news.
    if ((r.decision && r.decision !== "pending") || !r.document_id) continue;
    const t = at(r.requested_at);
    out.push({
      id: `rev:${r.review_id}`,
      kind: "decision",
      at: Number.isNaN(t) ? now : t,
      timed: !Number.isNaN(t),
      claim: `${kindWord(r.kind)} waits for your yes or no: “${cleanTitle(r.title ?? "")}”.`,
      source: "record",
      detail: r.space_name ? `For ${r.space_name}.` : "",
    });
  }
  for (const s of o.spaces) {
    const t = at(s.updated_at);
    const g = progressOf(s);
    if (Number.isNaN(t) || g.total === 0 || t < since) continue;
    out.push({
      id: `sp:${s.space_id}`,
      kind: "project",
      at: t,
      timed: true,
      claim: `${s.name} changed: ${g.done} of ${g.total} done.`,
      source: "record",
      detail:
        g.state === "waiting"
          ? "It waits for your decision."
          : g.state === "done"
            ? "Everything in it is done."
            : "Work goes on.",
      space: s.space_id,
    });
  }
  return out.sort((a, b) => (a.timed === b.timed ? b.at - a.at : a.timed ? 1 : -1));
}

export const storyFilter = (events: Event[], f: Filter): Event[] =>
  f === "all"
    ? events
    : events.filter((e) =>
        f === "problems" ? e.kind === "problem" : f === "decisions" ? e.kind === "decision" : e.kind === "project",
      );

/** A sentence of the lede and what proves it: an event, a filter of the timeline, or nothing. */
export type Ref = { event: string } | { filter: Filter };
export type Line = { text: string; ref?: Ref };

/** The day in up to three short sentences, each pointing at what proves it. */
export function ledeOf(events: Event[], o: { services: number; down: number; unknown: number }): Line[] {
  const out: Line[] = [];
  const problems = events.filter((e) => e.kind === "problem");
  const [first] = problems;
  if (first !== undefined) {
    out.push(
      problems.length === 1
        ? { text: first.claim, ref: { event: first.id } }
        : {
            text: `${first.claim.replace(/\.$/, "")}, and ${problems.length - 1} more ${problems.length === 2 ? "problem" : "problems"}.`,
            ref: { filter: "problems" },
          },
    );
  }
  const decisions = events.filter((e) => e.kind === "decision");
  const [one] = decisions;
  if (one !== undefined) {
    out.push(
      decisions.length === 1
        ? { text: "One decision waits for you.", ref: { event: one.id } }
        : { text: `${decisions.length} decisions wait for you.`, ref: { filter: "decisions" } },
    );
  }
  const up = o.services - o.down - o.unknown;
  if (o.services > 0) {
    const noun = (n: number) => (n === 1 ? "service is" : "services are");
    out.push({
      text:
        o.down + o.unknown === 0
          ? `All ${o.services} ${o.services === 1 ? "service is" : "services are"} up.`
          : `${o.down > 0 ? "The other " : ""}${up} ${noun(up)} up${o.unknown > 0 ? `, ${o.unknown} not known` : ""}.`,
    });
  }
  if (out.length === 0) out.push({ text: "A quiet day: nothing needs you." });
  return out;
}

/** When the newest check ran, as "40 seconds ago"; empty when no check has a time. */
export function lastCheck(lastRuns: (string | undefined)[], now = Date.now()): string {
  const ts = lastRuns.map(at).filter((x) => !Number.isNaN(x));
  if (ts.length === 0) return "";
  const s = Math.max(0, Math.round((now - Math.max(...ts)) / 1000));
  const n = (v: number, one: string) => `${v} ${one}${v === 1 ? "" : "s"} ago`;
  return s < 60 ? n(s, "second") : s < 3600 ? n(Math.round(s / 60), "minute") : n(Math.round(s / 3600), "hour");
}

/** The last checks, oldest first, so a strip reads left to right like time. */
export const stripOf = (runs: Run[], n = 20): Run[] =>
  runs
    .slice()
    .sort((a, b) => Date.parse(b.at) - Date.parse(a.at))
    .slice(0, n)
    .reverse();

/** "4 of the last 14 checks failed, the last answer was 503": what the strip shows, said plainly. */
export function stripLine(strip: Run[]): string {
  const last = strip[strip.length - 1];
  if (last === undefined) return "No checks have run yet.";
  const failed = strip.filter((r) => !r.ok).length;
  const head =
    failed === 0
      ? `All of the last ${strip.length} checks passed`
      : `${failed} of the last ${strip.length} checks failed`;
  return `${head}${last.status_code ? `, the last answer was ${last.status_code}` : ""}.`;
}

/** "Today", "Yesterday" or the date, for the timeline's day marks. */
export function dayOf(ms: number, now = Date.now()): string {
  const d = new Date(ms);
  if (d.toDateString() === new Date(now).toDateString()) return "Today";
  if (d.toDateString() === new Date(now - 86_400_000).toDateString()) return "Yesterday";
  return d.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" });
}

/** True when the account has nothing at all to show: a brand-new account says so instead of an empty story. */
export const isFirstRun = (o: { decisions: number; services: number; projects: number }): boolean =>
  o.decisions + o.services + o.projects === 0;
