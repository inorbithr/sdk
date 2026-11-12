// Consent for sensors, in the sdk UI (PRD 0077, PRD 0117; RFC 0217). Explicit and per person: a person
// controls the sensors they own, and only those. The service is the record: a switch is the sensor's
// state (active reads and sends; paused does not; revoked is permanent), so there is nothing kept in
// the page that could disagree with it, and the server decides who may act.
//
// The Mac app's own local checks (CPU, memory, thermal, disk) are read by the Mac app and never reach
// a browser; the web sheet manages what the service holds for this person.

import type { Sensor } from "./sensors-core.ts";

/** Only a strict `active` is on: unknown, missing and paused words are off. */
export const isOn = (s: Pick<Sensor, "state">): boolean => s.state === "active";

/** The sensors a person owns: their subject matches the sensor's owner, and built-ins (derived by the service) are never theirs to switch. */
export function mineOf(sensors: readonly Sensor[], subject: string | null | undefined): Sensor[] {
  const me = (subject ?? "").trim().toLowerCase();
  if (!me) return [];
  return sensors.filter((s) => !s.builtin && (s.owner ?? "").trim().toLowerCase() === me);
}

/** One switch's wire action: off is a pause, on a resume. A revoked sensor has none. */
export function switchAction(s: Pick<Sensor, "state">, on: boolean): "pause" | "resume" | null {
  if (s.state === "revoked") return null;
  if (on) return s.state === "active" ? null : "resume";
  return s.state === "active" ? "pause" : null;
}

/** What "Pause all of mine" would pause: every owned sensor that is on. */
export const pauseAllTargets = (mine: readonly Sensor[]): Sensor[] => mine.filter(isOn);

export const stateWord = (s: Pick<Sensor, "state">): string =>
  s.state === "active" ? "On" : s.state === "paused" ? "Paused" : "Revoked";

/** What was sent: GET /v1/atlas/sensors/{id}/sent. */
export type WireSent = {
  sensor_id?: string;
  days?: { day?: string; readings?: number | string; digests?: number | string }[];
  readings?: number | string;
  rollups?: number | string;
  first_at?: string;
  last_at?: string;
};

const num = (v: number | string | undefined): number => {
  const n = typeof v === "string" ? Number(v) : (v ?? 0);
  return Number.isFinite(n) ? n : 0;
};

/** "12 readings and 3 rollups since 2026-11-01", or that nothing was sent. 64-bit counts arrive as strings. */
export function sentText(sent: WireSent | null | undefined): string {
  if (!sent) return "";
  const readings = num(sent.readings);
  const rollups = num(sent.rollups);
  if (readings === 0 && rollups === 0) return "Nothing sent yet.";
  const part = (n: number, w: string) => `${n} ${w}${n === 1 ? "" : "s"}`;
  const since =
    sent.first_at && Number.isFinite(Date.parse(sent.first_at)) ? ` since ${sent.first_at.slice(0, 10)}` : "";
  return `${[readings ? part(readings, "reading") : "", rollups ? part(rollups, "rollup") : ""].filter(Boolean).join(" and ")}${since}.`;
}
