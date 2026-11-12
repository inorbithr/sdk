// The page's view of who it acts as (RFC 0217, round 4): read once, again after a sign-in or sign-out, and every
// two seconds while the person's browser is signing in. The page holds no credential, only what `/api/session` says.
import { useCallback, useEffect, useState } from "react";
import { readSession, type SessionState, signIn, signOut } from "./api.ts";

const WAIT_MS = 2000;

export function useSession(): { session: SessionState | null; busy: boolean; signIn: () => void; signOut: () => void } {
  const [session, setSession] = useState<SessionState | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => setSession(await readSession()), []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const waiting = session !== null && session.state !== "unreachable" && session.login === "waiting";
  useEffect(() => {
    if (!waiting) return;
    const id = window.setInterval(() => void refresh(), WAIT_MS);
    return () => window.clearInterval(id);
  }, [waiting, refresh]);

  const act = (run: () => Promise<boolean>) => async () => {
    setBusy(true);
    await run();
    await refresh();
    setBusy(false);
  };

  return { session, busy, signIn: act(signIn), signOut: act(signOut) };
}
