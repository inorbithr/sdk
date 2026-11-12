// What Settings reads, and how (RFC 0217, slice 4): plain GETs through the signed-in local server, one per pane that
// shows the account's own records. The same `Part` Home uses, so a pane says loading, signed out or why it failed.
import { useCallback, useEffect, useState } from "react";
import { apiGet } from "./api.ts";
import type { Part } from "./home-data.ts";
import { type Call, failure, type Invite, offlineWhy } from "./team-core.ts";
import { apiSend } from "./team-send.ts";

const enc = encodeURIComponent;

export type Member = {
  user_id?: string;
  email?: string;
  name?: string;
  role?: string;
  mfa?: string;
  last_seen_at?: string;
};

/** The reads the panes make for an account (an org id). */
export const settingsReads = (account: string) => ({
  members: { path: `/v1/accounts/orgs/${enc(account)}/members`, query: {} as Record<string, string> },
  invitations: { path: `/v1/accounts/orgs/${enc(account)}/invitations`, query: {} as Record<string, string> },
});

/** One read, again on `again`. `null` means nothing to ask yet (not signed in), so nothing is asked. */
export function useRead<W, T>(
  read: { path: string; query: Record<string, string> } | null,
  pick: (wire: W) => T,
): Part<T> & { again: () => void } {
  const [part, setPart] = useState<Part<T>>({ state: "loading" });
  const [n, setN] = useState(0);
  const key = read === null ? "" : `${read.path}?${JSON.stringify(read.query)}`;
  // `pick` is a fresh function each render; the read is named by its path and query, so only those restart it.
  // biome-ignore lint/correctness/useExhaustiveDependencies: see above
  useEffect(() => {
    if (read === null) return;
    let live = true;
    setPart({ state: "loading" });
    void apiGet<W>(read.path, read.query).then((r) => {
      if (!live) return;
      setPart(
        r.state === "ok"
          ? { state: "ok", data: pick(r.data) }
          : r.state === "signed-out"
            ? { state: "signed-out" }
            : { state: "error", reason: r.reason },
      );
    });
    return () => {
      live = false;
    };
  }, [key, n]);
  const again = useCallback(() => setN((x) => x + 1), []);
  return { ...part, again };
}

export type { Invite };

/** True while the browser has a connection; changes with the browser's online and offline events. */
const connected = () => (typeof navigator === "undefined" ? true : navigator.onLine);
export function useOnline(): boolean {
  const [on, setOn] = useState(connected);
  useEffect(() => {
    const f = () => setOn(connected());
    window.addEventListener("online", f);
    window.addEventListener("offline", f);
    return () => {
      window.removeEventListener("online", f);
      window.removeEventListener("offline", f);
    };
  }, []);
  return on;
}

/** Sends one team call and says how it went in words; `after` runs only when the API took it. */
export function useWrites() {
  const [working, setBusy] = useState(false);
  const why = offlineWhy(useOnline());
  const busy = working || why !== null; // offline disables every write control, like a write in flight
  const [note, setNote] = useState<{ ok: boolean; text: string } | null>(null);
  const run = useCallback(
    async (call: Call, what: string, after?: () => void) => {
      if (why !== null) {
        setNote({ ok: false, text: why });
        return;
      }
      setBusy(true);
      setNote(null);
      const r = await apiSend(call.method, call.path, call.body);
      setBusy(false);
      if (r.state === "ok") {
        setNote({ ok: true, text: `Done: ${what}.` });
        after?.();
      } else {
        setNote({
          ok: false,
          text: r.state === "signed-out" ? "Sign in again to do that." : failure(r.status, r.reason, what),
        });
      }
    },
    [why],
  );
  return { busy, offline: why, note, run, clear: () => setNote(null) };
}
