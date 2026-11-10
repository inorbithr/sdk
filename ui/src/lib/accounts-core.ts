// The account name, the account list and the switcher menu (RFC 0217.1, slice 1). Pure: no React, no DOM, so node
// tests it. The list is `GET /v1/accounts/me` (the person's own account and their teams); the menu is the Mac app's
// switcher drawn with the frame's one context menu: accounts with plan and role, then projects with their
// environment, then sign in or out. No row ever shows an id: a missing name falls back to words.
import { type MenuEntry, SEP } from "../frame/context-menu-core.ts";
import type { SessionState } from "./api.ts";

type AccountWire = { id?: string; kind?: string; slug?: string; name?: string; plan?: string; role?: string };
/** `GET /v1/accounts/me`, as far as the page reads it. */
export type MeWire = {
  email?: string;
  name?: string;
  account?: AccountWire;
  teams?: AccountWire[];
  preferences?: { open_account_id?: string; default_account_id?: string; last_account_id?: string };
};

export type AccountRow = { id: string; name: string; kind: "personal" | "team"; plan: string; role: string };

const str = (v: unknown): string => (typeof v === "string" ? v : "");

const row = (a: AccountWire, kind: "personal" | "team"): AccountRow | null => {
  const id = str(a.id);
  if (id === "") return null;
  const fallback = kind === "personal" ? "Personal account" : "Team";
  return { id, name: str(a.name) || str(a.slug) || fallback, kind, plan: str(a.plan), role: str(a.role) };
};

/** The person's own account first, then their teams by name. A row with no id cannot be switched to, so it is left out. */
export function accountRows(me: MeWire | null): AccountRow[] {
  if (me === null) return [];
  const own = me.account === undefined ? null : row(me.account, "personal");
  const teams = (me.teams ?? [])
    .map((t) => row(t, "team"))
    .filter((r): r is AccountRow => r !== null)
    .sort((a, b) => a.name.localeCompare(b.name));
  return own === null ? teams : [own, ...teams];
}

/** This tab's pick while it still exists, else the account to open, else the one the sign-in is for, else the first. */
export function pickActive(
  rows: AccountRow[],
  picked: string | null,
  open: string | null,
  token: string,
): string | null {
  const has = (id: string | null) => id !== null && rows.some((r) => r.id === id);
  if (has(picked)) return picked;
  if (has(open)) return open;
  if (has(token)) return token;
  return rows[0]?.id ?? null;
}

/** What the header says: the account's name, or words while the list is not there. Never an id. */
export function accountName(rows: AccountRow[], active: string | null): string {
  return rows.find((r) => r.id === active)?.name ?? "Your account";
}

const title = (s: string, fallback: string) => (s === "" ? fallback : `${s.charAt(0).toUpperCase()}${s.slice(1)}`);
export const planLabel = (plan: string) => title(plan, "Free");
export const roleLabel = (role: string) => title(role, "Member");

/** The session as the active account sees it: every read keys on `session.account`, so one swap moves them all. */
export function withAccount(session: SessionState | null, active: string | null): SessionState | null {
  if (session === null || session.state !== "signed-in" || active === null || active === session.account)
    return session;
  return { ...session, account: active };
}

export type SwitcherInput = {
  rows: AccountRow[];
  active: string | null;
  projects: { id: string; name: string }[];
  /** The CLI environment this page is signed in to (the only one the page can use). */
  profile: string;
  signedIn: boolean;
  switchTo: (id: string) => void;
  signIn: () => void;
  signOut: () => void;
};

const NEW_PROJECT = "Start a project from a folder with `iohr init`; the page cannot create one yet.";
const NEW_ENV = "Environments are built into the CLI (production and local); none can be added here.";

export function accountMenu(i: SwitcherInput): MenuEntry[] {
  if (!i.signedIn) return [{ id: "session:in", label: "Sign in…", icon: "log-in", run: i.signIn }];
  const out: MenuEntry[] = [];
  if (i.rows.length > 0) {
    out.push({ heading: "Accounts" });
    for (const r of i.rows) {
      const current = r.id === i.active;
      out.push({
        id: `account:${r.id}`,
        label: r.name,
        hint: `${r.kind === "personal" ? "Personal" : planLabel(r.plan)} · ${roleLabel(r.role)}`,
        checked: current,
        run: () => {
          if (!current) i.switchTo(r.id);
        },
      });
    }
    out.push(SEP);
  }
  out.push({ heading: "Projects" });
  for (const p of i.projects) {
    out.push({
      id: `project:${p.id}`,
      label: p.name,
      icon: "folder",
      items: [{ id: `env:${p.id}`, label: i.profile === "" ? "production" : i.profile, checked: true }],
    });
  }
  out.push(
    { id: "project:new", label: "New project…", icon: "plus", disabled: NEW_PROJECT },
    { id: "project:env", label: "Add environment…", icon: "plus", disabled: NEW_ENV },
    SEP,
    { id: "session:out", label: "Sign out", icon: "log-out", run: i.signOut },
  );
  return out;
}
