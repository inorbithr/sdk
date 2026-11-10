// Which account this tab acts as (RFC 0217.1, slice 1). As in the console, the pick belongs to the tab
// (sessionStorage), so two tabs can sit on two accounts; the person's own default is only the start. Storage can be
// missing or refused: the pick then lasts for this load.
import { useCallback, useSyncExternalStore } from "react";
import { accountRows, type MeWire, pickActive, withAccount } from "./accounts-core.ts";
import type { SessionState } from "./api.ts";
import { ME_READ, type Part } from "./home-data.ts";
import { useRead } from "./settings-data.ts";
import { rememberAccount } from "./team-core.ts";
import { apiSend } from "./team-send.ts";

const KEY = "iohr.account";
let picked: string | null = null;
try {
  picked = sessionStorage.getItem(KEY);
} catch {
  // no storage: the pick lasts for this load
}
const subs = new Set<() => void>();
const subscribe = (f: () => void) => {
  subs.add(f);
  return () => {
    subs.delete(f);
  };
};
const get = () => picked;

function pick(id: string) {
  picked = id;
  try {
    sessionStorage.setItem(KEY, id);
  } catch {
    // see above
  }
  for (const f of subs) f();
}

/** The accounts the person can act as, which one this tab does, and the session as that account sees it. */
export function useAccounts(raw: SessionState | null) {
  const signedIn = raw?.state === "signed-in";
  const me: Part<MeWire> & { again: () => void } = useRead<MeWire, MeWire>(signedIn ? ME_READ : null, (w) => w);
  const tab = useSyncExternalStore(subscribe, get, get);
  const rows = accountRows(me.state === "ok" ? me.data : null);
  const open = me.state === "ok" ? (me.data.preferences?.open_account_id ?? null) : null;
  const token = raw?.state === "signed-in" ? raw.account : "";
  const active = rows.length === 0 ? (token === "" ? null : token) : pickActive(rows, tab, open, token);
  // The tab switches at once; the API is told afterwards, as the console does, so the next sign-in opens this
  // account. A refusal changes nothing the person sees: the pick is the tab's.
  const switchTo = useCallback((id: string) => {
    pick(id);
    const c = rememberAccount(id);
    void apiSend(c.method, c.path, c.body);
  }, []);
  return { me, rows, active, session: withAccount(raw, active), switchTo };
}

/** What `useAccounts` gives the frame and Settings. */
export type AccountsState = ReturnType<typeof useAccounts>;
