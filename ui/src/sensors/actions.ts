// What a person can do from the sensors page, each through the API with their own sign-in: ask to view a session,
// pause, resume or revoke a sensor of theirs, read or delete what a sensor sent. The services decide who may; the
// page only says plainly what happened.
import { apiGet, apiSend, type Read } from "../lib/api.ts";
import type { WireSent } from "../lib/sensors-consent.ts";

export type Done = { ok: boolean; text: string };

type Sent = Awaited<ReturnType<typeof apiSend>>;
const sentOk = (r: Sent): boolean => r.state === "ok";
const why = (r: Sent, what: string): string =>
  r.state === "ok" ? what : r.state === "signed-out" ? "You are signed out: sign in again." : `${what}: ${r.reason}`;

/** Asks the session's owner to let this person watch it; the owner answers in their own app. */
export async function askToView(threadId: string | null): Promise<Done> {
  if (!threadId) return { ok: false, text: "This one has no thread to ask about." };
  const r = await apiSend("POST", `/v1/coordinator/threads/${encodeURIComponent(threadId)}/view-requests`, {});
  if (sentOk(r)) return { ok: true, text: "Asked. Its owner sees your request and can say yes." };
  if (r.state === "error" && r.status === 404)
    return { ok: false, text: "Asking to view is not available: the coordinator does not take view requests here." };
  return { ok: false, text: why(r, "Could not ask") };
}

export type Change = "pause" | "resume" | "revoke";
const DONE: Record<Change, string> = {
  pause: "Paused. It sends nothing until you turn it on.",
  resume: "On again.",
  revoke: "Revoked. It stops sending at once, for good.",
};

/** Pauses, resumes or revokes one sensor of the person's. */
export async function changeSensor(id: string, what: Change): Promise<Done> {
  const r = await apiSend("POST", `/v1/atlas/sensors/${encodeURIComponent(id)}/${what}`, {});
  return sentOk(r) ? { ok: true, text: DONE[what] } : { ok: false, text: why(r, `Could not ${what}`) };
}

/** What the platform keeps of what a sensor sent. */
export async function readSent(id: string): Promise<Read<WireSent>> {
  return apiGet<WireSent>(`/v1/atlas/sensors/${encodeURIComponent(id)}/sent`, {});
}

/** Deletes everything a sensor sent (readings and roll-ups); the sensor itself stays. */
export async function deleteSent(id: string): Promise<Done> {
  const r = await apiSend("DELETE", `/v1/atlas/sensors/${encodeURIComponent(id)}/sent`);
  return sentOk(r) ? { ok: true, text: "Deleted what it sent." } : { ok: false, text: why(r, "Could not delete") };
}
