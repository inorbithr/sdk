import { type ReactNode, useState } from "react";
import { resetAll } from "./frame/arrange.ts";
import { cn } from "./frame/cn.ts";
import { closeDock, openDock } from "./frame/dock-frame.ts";
import { FRAME_SHORTCUTS, LAYOUT_SHORTCUTS, symbols } from "./frame/shortcuts.ts";
import { type SidebarMode, setMode } from "./frame/sidebar-frame.ts";
import { type AccountRow, planLabel, roleLabel } from "./lib/accounts-core.ts";
import type { AccountsState } from "./lib/accounts-store.ts";
import type { SessionState } from "./lib/api.ts";
import { updateArrangement, useArrangement } from "./lib/arrange-store.ts";
import { updateDock, useDock } from "./lib/dock-store.ts";
import type { Listing } from "./lib/extensions.ts";
import { entryOf, FEATURES, isDown } from "./lib/features-core.ts";
import { allOnline, backOnline, undoLast, useFeatures, useUndo } from "./lib/features-store.ts";
import type { Space } from "./lib/home-core.ts";
import { homeReads } from "./lib/home-data.ts";
import { HELD, heldNow, type Pane } from "./lib/settings-core.ts";
import { type Invite, type Member, settingsReads, useRead, useWrites } from "./lib/settings-data.ts";
import { updateSidebar, useSidebar } from "./lib/sidebar-store.ts";
import {
  changeRole,
  createTeam,
  defaultAccount,
  emailOk,
  GRANTABLE,
  invite,
  isOpen,
  manages,
  memberRules,
  openInvites,
  personName,
  removeMember,
  renameTeam,
  resendInvite,
  revokeInvite,
  roleWord,
  teamNameOk,
} from "./lib/team-core.ts";
import { setThemeChoice, useThemeChoice } from "./lib/theme.ts";
import { CHOICES } from "./lib/theme-core.ts";

// The bodies of Settings' panes (RFC 0217, slice 4). Each shows what this page really has: a control that changes the
// real state, or a read of the signed-in account, or a plain line that says it is empty or not built here. Nothing is
// filled in to look like data.

const card = "bg-card text-card-foreground rounded-2xl border shadow-xs";
const btn =
  "hover:bg-accent hover:text-foreground rounded-md border px-2.5 py-1 text-[13px] disabled:opacity-50 outline-none focus-visible:ring-2 ring-ring";
const muted = "text-muted-foreground";
const field =
  "bg-background rounded-md border px-2 py-1 text-[13px] outline-none focus-visible:ring-2 ring-ring min-w-0 disabled:opacity-50";

export type PaneProps = {
  session: SessionState | null;
  /** The accounts this person can act as and the one this tab does (RFC 0217.1). */
  accounts: AccountsState;
  busy: boolean;
  onSignIn: () => void;
  onSignOut: () => void;
  listing: Listing | null;
  onEdit: (id: string) => void;
};

function Section({ title, children, note }: { title: string; children: ReactNode; note?: string }) {
  return (
    <section className={cn(card, "p-4")}>
      <h3 className="mb-2 text-sm font-semibold">{title}</h3>
      {note ? <p className={cn(muted, "mb-2 text-[13px]")}>{note}</p> : null}
      {children}
    </section>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex min-h-9 items-center justify-between gap-3 border-t py-1.5 text-[13px] first:border-t-0">
      <span className="min-w-0">{label}</span>
      <span className="flex min-w-0 items-center gap-2">{children}</span>
    </div>
  );
}

/** A read's three non-answers, in plain words; null when there is data to show. */
function Why({
  part,
  what,
  onSignIn,
  onRetry,
}: {
  part: { state: string; reason?: string };
  what: string;
  onSignIn: () => void;
  onRetry: () => void;
}) {
  if (part.state === "ok") return null;
  if (part.state === "loading") return <p className={cn(muted, "text-[13px]")}>Loading {what}…</p>;
  if (part.state === "signed-out")
    return (
      <p className={cn(muted, "flex items-center gap-2 text-[13px]")} data-pane-state="signed-out">
        Sign in to see {what}.
        <button type="button" className={btn} onClick={onSignIn}>
          Sign in
        </button>
      </p>
    );
  return (
    <p className="text-destructive flex flex-wrap items-center gap-2 text-[13px]" role="alert" data-pane-state="error">
      Could not read {what}: {part.reason}
      <button type="button" className={btn} onClick={onRetry}>
        Try again
      </button>
    </p>
  );
}

const accountOf = (s: SessionState | null): string | null =>
  s?.state === "signed-in" && s.account !== "" ? s.account : null;

export function General() {
  const [sidebar] = useSidebar();
  const [dock] = useDock();
  const arr = useArrangement();
  const modes: SidebarMode[] = ["expanded", "rail", "hidden"];
  const laidOut = arr.dock !== null || arr.before !== null || arr.order[0] !== "page";
  return (
    <>
      <Section title="Sidebar">
        <Row label="Sidebar">
          <div role="radiogroup" aria-label="Sidebar" className="flex gap-1">
            {modes.map((m) => (
              <label
                key={m}
                className={cn(btn, "cursor-pointer capitalize", sidebar.mode === m && "bg-accent text-foreground")}
              >
                <input
                  type="radio"
                  name="sidebar"
                  className="sr-only"
                  checked={sidebar.mode === m}
                  onChange={() => updateSidebar((s) => setMode(s, m))}
                />
                {m}
              </label>
            ))}
          </div>
        </Row>
        <Row label="Details dock">
          <span className={muted}>{dock.open ? "Open" : "Closed"}</span>
          <button
            type="button"
            className={btn}
            onClick={() => {
              if (dock.open) {
                updateDock(closeDock);
                updateArrangement((a) => (a.dock === null ? a : { ...a, dock: null }));
              } else updateDock(openDock);
            }}
          >
            {dock.open ? "Close" : "Open"}
          </button>
        </Row>
      </Section>
      <Section title="Window layout" note="Chosen from the layout button at the top of any page.">
        <Row label={laidOut ? "A layout is applied" : "Full window"}>
          <button type="button" className={btn} disabled={!laidOut} onClick={() => updateArrangement(resetAll)}>
            Reset layout
          </button>
        </Row>
      </Section>
    </>
  );
}

export function Appearance() {
  const choice = useThemeChoice();
  return (
    <Section title="Theme" note="System follows the setting of this computer and changes with it.">
      <div role="radiogroup" aria-label="Theme" className="flex gap-1">
        {CHOICES.map((c) => (
          <label
            key={c.id}
            className={cn(btn, "cursor-pointer", choice === c.id && "bg-accent text-foreground font-medium")}
            data-theme-choice={c.id}
          >
            <input
              type="radio"
              name="theme"
              className="sr-only"
              checked={choice === c.id}
              onChange={() => setThemeChoice(c.id)}
            />
            {c.label}
          </label>
        ))}
      </div>
    </Section>
  );
}

export function Accounts({ session, accounts, busy, onSignIn, onSignOut }: PaneProps) {
  if (session === null) return <p className={cn(muted, "text-[13px]")}>Checking sign-in…</p>;
  if (session.state === "unreachable")
    return (
      <p className="text-destructive text-[13px]" role="alert">
        {session.reason}
      </p>
    );
  const current: AccountRow | undefined = accounts.rows.find((r) => r.id === accounts.active);
  const me = accounts.me.state === "ok" ? accounts.me.data : null;
  const who = me?.email || me?.name || "";
  const signedIn = session.state === "signed-in";
  return (
    <>
      <Section
        title={session.state === "signed-in" ? "Signed in" : "Not signed in"}
        note="Signing in is the same as `iohr login`: your browser opens, and the local server keeps the credential. This page never holds it."
      >
        {session.state === "signed-in" ? (
          <>
            <Row label="Profile">{session.profile}</Row>
            <Row label="Kind">{session.kind}</Row>
            <Row label="Account">{current?.name ?? "Your account"}</Row>
            {current ? <Row label="Plan">{planLabel(current.plan)}</Row> : null}
            {current ? <Row label="Your role">{roleLabel(current.role)}</Row> : null}
            {who ? <Row label="Signed in as">{who}</Row> : null}
            <Row label="Scopes">
              <span className="text-right">{session.scopes.length === 0 ? "none" : session.scopes.join(", ")}</span>
            </Row>
            <div className="pt-2">
              <button type="button" className={btn} disabled={busy} onClick={onSignOut}>
                Sign out
              </button>
            </div>
          </>
        ) : (
          <div className="flex flex-wrap items-center gap-2">
            {session.login === "failed" ? (
              <span className="text-destructive text-[13px]" role="alert">
                Sign-in failed{session.reason ? `: ${session.reason}` : ""}
              </span>
            ) : null}
            <button type="button" className={btn} disabled={busy || session.login === "waiting"} onClick={onSignIn}>
              {session.login === "waiting" ? "Finish in your browser…" : "Sign in"}
            </button>
          </div>
        )}
      </Section>
      {signedIn ? <TeamsSection accounts={accounts} /> : null}
    </>
  );
}

/** Rename this team, start another, and choose which account opens first (the console's Settings, through the API). */
function TeamsSection({ accounts }: { accounts: AccountsState }) {
  const w = useWrites();
  const [name, setName] = useState("");
  const [fresh, setFresh] = useState("");
  const current = accounts.rows.find((r) => r.id === accounts.active);
  const prefs = accounts.me.state === "ok" ? accounts.me.data.preferences : undefined;
  const first = prefs?.default_account_id ?? prefs?.open_account_id ?? "";
  const mayRename = current?.kind === "team" && manages(current.role);
  return (
    <Section title="Teams" note="The same changes as the console's Settings, made through the API as you.">
      {current?.kind === "team" ? (
        <Row label="Team name">
          {mayRename ? (
            <form
              className="flex items-center gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                if (accounts.active === null) return;
                void w.run(renameTeam(accounts.active, name), "rename the team", () => {
                  setName("");
                  accounts.me.again();
                });
              }}
            >
              <input
                className={field}
                aria-label="New team name"
                placeholder={current.name}
                value={name}
                maxLength={80}
                onChange={(e) => setName(e.target.value)}
              />
              <button type="submit" className={btn} disabled={w.busy || !teamNameOk(name)}>
                Rename
              </button>
            </form>
          ) : (
            <span className={muted}>Only an owner or admin renames the team.</span>
          )}
        </Row>
      ) : null}
      <Row label="New team">
        <form
          className="flex items-center gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            void w.run(createTeam(fresh), "create the team", () => {
              setFresh("");
              accounts.me.again();
            });
          }}
        >
          <input
            className={field}
            aria-label="Name of the new team"
            placeholder="Team name"
            value={fresh}
            maxLength={80}
            onChange={(e) => setFresh(e.target.value)}
          />
          <button type="submit" className={btn} disabled={w.busy || !teamNameOk(fresh)}>
            Create team
          </button>
        </form>
      </Row>
      <Row label="Opens first">
        <select
          className={field}
          aria-label="The account that opens first"
          value={accounts.rows.some((r) => r.id === first) ? first : ""}
          disabled={w.busy}
          onChange={(e) => {
            if (e.target.value !== "")
              void w.run(defaultAccount(e.target.value), "save the account that opens first", accounts.me.again);
          }}
        >
          <option value="" disabled>
            Not chosen
          </option>
          {accounts.rows.map((r) => (
            <option key={r.id} value={r.id}>
              {r.name}
            </option>
          ))}
        </select>
      </Row>
      <WriteNote w={w} />
    </Section>
  );
}

/** What the last write said, in words; a failure is an alert. */
function WriteNote({ w }: { w: ReturnType<typeof useWrites> }) {
  if (w.note === null) return null;
  return (
    <p
      className={cn("pt-2 text-[13px]", w.note.ok ? muted : "text-destructive")}
      role={w.note.ok ? "status" : "alert"}
      data-write={w.note.ok ? "ok" : "failed"}
    >
      {w.note.text}
    </p>
  );
}

const when = (iso?: string) => {
  const t = iso ? Date.parse(iso) : Number.NaN;
  return Number.isNaN(t)
    ? "never"
    : new Date(t).toLocaleDateString([], { year: "numeric", month: "short", day: "numeric" });
};

export function Team({ session, accounts, onSignIn }: PaneProps) {
  const account = accountOf(session);
  const current = accounts.rows.find((r) => r.id === accounts.active);
  const mine = current?.role;
  const may = manages(mine);
  const selfId = accounts.me.state === "ok" ? (accounts.me.data.user_id ?? "") : "";
  const members = useRead<{ members?: Member[] }, Member[]>(
    account === null ? null : settingsReads(account).members,
    (w) => w.members ?? [],
  );
  const invites = useRead<{ invitations?: Invite[] }, Invite[]>(
    account === null || !may || current?.kind !== "team" ? null : settingsReads(account).invitations,
    (w) => w.invitations ?? [],
  );
  const w = useWrites();
  const [sure, setSure] = useState<string | null>(null);
  const [email, setEmail] = useState("");
  const [role, setRole] = useState("member");
  if (session === null) return <p className={cn(muted, "text-[13px]")}>Checking sign-in…</p>;
  if (session.state === "signed-out")
    return <Why part={{ state: "signed-out" }} what="your team" onSignIn={onSignIn} onRetry={() => {}} />;
  if (account === null)
    return <p className={cn(muted, "text-[13px]")}>This sign-in has no account, so there is no team to list.</p>;
  if (current?.kind === "personal")
    return (
      <p className={cn(muted, "text-[13px]")} data-pane-state="personal">
        A personal account has no team. Start one under Accounts, or switch to a team from the menu at the top.
      </p>
    );
  if (members.state !== "ok")
    return <Why part={members} what="your team" onSignIn={onSignIn} onRetry={members.again} />;
  const orgId = account;
  const after = () => {
    setSure(null);
    members.again();
    invites.again();
  };
  const list = members.data;
  return (
    <>
      <section className={cn(card, "overflow-x-auto p-2")} aria-label="Members">
        {list.length === 0 ? (
          <p className={cn(muted, "px-2 py-1 text-[13px]")}>The account lists no members.</p>
        ) : (
          <table className="w-full text-left text-[13px]">
            <thead className={muted}>
              <tr>
                <th className="px-2 py-1 font-medium">Name</th>
                <th className="px-2 py-1 font-medium">Role</th>
                <th className="px-2 py-1 font-medium">Second factor</th>
                <th className="px-2 py-1 font-medium">Last seen</th>
                <th className="px-2 py-1" />
              </tr>
            </thead>
            <tbody>
              {list.map((m) => {
                const rules = memberRules(m, selfId, mine);
                const uid = m.user_id ?? "";
                const who = personName(m);
                const confirming = sure === uid && uid !== "";
                return (
                  <tr key={uid || m.email} className="border-t" data-member>
                    <td className="px-2 py-1.5">
                      <div className="truncate">
                        {who}
                        {uid !== "" && uid === selfId ? <span className={muted}> (you)</span> : null}
                      </div>
                      {m.name && m.email ? <div className={cn(muted, "truncate text-[12px]")}>{m.email}</div> : null}
                    </td>
                    <td className="px-2 py-1.5">
                      {rules.changeRole ? (
                        <select
                          className={field}
                          aria-label={`Role of ${who}`}
                          value={m.role === "admin" ? "admin" : "member"}
                          disabled={w.busy}
                          onChange={(e) =>
                            void w.run(changeRole(orgId, uid, e.target.value), `change ${who}'s role`, after)
                          }
                        >
                          {GRANTABLE.map((r) => (
                            <option key={r.id} value={r.id}>
                              {r.label}
                            </option>
                          ))}
                        </select>
                      ) : (
                        roleWord(m.role)
                      )}
                    </td>
                    <td className="px-2 py-1.5">{m.mfa || "unknown"}</td>
                    <td className="px-2 py-1.5 whitespace-nowrap">{when(m.last_seen_at)}</td>
                    <td className="px-2 py-1.5 text-right whitespace-nowrap">
                      {rules.remove || rules.leave ? (
                        confirming ? (
                          <>
                            <button
                              type="button"
                              className={cn(btn, "text-destructive")}
                              disabled={w.busy}
                              onClick={() =>
                                void w.run(
                                  removeMember(orgId, uid),
                                  rules.leave ? "leave the team" : `remove ${who}`,
                                  after,
                                )
                              }
                            >
                              {rules.leave ? "Yes, leave" : "Yes, remove"}
                            </button>{" "}
                            <button type="button" className={btn} onClick={() => setSure(null)}>
                              Cancel
                            </button>
                          </>
                        ) : (
                          <button type="button" className={btn} disabled={w.busy} onClick={() => setSure(uid)}>
                            {rules.leave ? "Leave" : "Remove"}
                          </button>
                        )
                      ) : null}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </section>
      {may && current?.kind === "team" ? (
        <Section
          title="Invitations"
          note="An invitation is a mailed link. Sending it again makes a new link and ends the old one."
        >
          {invites.state === "ok" ? (
            openInvites(invites.data).length === 0 ? (
              <p className={cn(muted, "text-[13px]")}>No invitations yet.</p>
            ) : (
              openInvites(invites.data).map((i) => (
                <Row key={i.id ?? i.email} label={i.email ?? "Unknown address"}>
                  <span className={muted} data-invite={isOpen(i) ? "open" : "closed"}>
                    {roleWord(i.role)} · {i.status || "pending"}
                  </span>
                  {isOpen(i) && i.id ? (
                    <>
                      <button
                        type="button"
                        className={btn}
                        disabled={w.busy}
                        onClick={() =>
                          void w.run(resendInvite(orgId, i.id ?? ""), `resend the invitation to ${i.email}`, after)
                        }
                      >
                        Resend
                      </button>
                      <button
                        type="button"
                        className={btn}
                        disabled={w.busy}
                        onClick={() =>
                          void w.run(revokeInvite(orgId, i.id ?? ""), `withdraw the invitation to ${i.email}`, after)
                        }
                      >
                        Withdraw
                      </button>
                    </>
                  ) : null}
                </Row>
              ))
            )
          ) : (
            <Why part={invites} what="the invitations" onSignIn={onSignIn} onRetry={invites.again} />
          )}
          <form
            className="flex flex-wrap items-center gap-2 pt-3"
            onSubmit={(e) => {
              e.preventDefault();
              void w.run(invite(orgId, email, role), `invite ${email.trim()}`, () => {
                setEmail("");
                after();
              });
            }}
          >
            <input
              className={cn(field, "flex-1")}
              type="email"
              aria-label="E-mail address to invite"
              placeholder="name@example.com"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
            />
            <select className={field} aria-label="Role to give" value={role} onChange={(e) => setRole(e.target.value)}>
              {GRANTABLE.map((r) => (
                <option key={r.id} value={r.id}>
                  {r.label}
                </option>
              ))}
            </select>
            <button type="submit" className={btn} disabled={w.busy || !emailOk(email)}>
              Send invitation
            </button>
          </form>
        </Section>
      ) : null}
      {!may ? (
        <p className={cn(muted, "text-[13px]")}>Only an owner or admin invites people or changes roles.</p>
      ) : null}
      <WriteNote w={w} />
    </>
  );
}

export function Projects({ session, onSignIn }: PaneProps) {
  const account = accountOf(session);
  const spaces = useRead<{ spaces?: Space[] }, Space[]>(
    account === null ? null : { path: homeReads(account).spaces.path, query: homeReads(account).spaces.query },
    (w) => w.spaces ?? [],
  );
  if (session === null) return <p className={cn(muted, "text-[13px]")}>Checking sign-in…</p>;
  if (session.state === "signed-out")
    return <Why part={{ state: "signed-out" }} what="your projects" onSignIn={onSignIn} onRetry={() => {}} />;
  if (account === null)
    return <p className={cn(muted, "text-[13px]")}>This sign-in has no account, so there are no projects to list.</p>;
  if (spaces.state !== "ok")
    return <Why part={spaces} what="your projects" onSignIn={onSignIn} onRetry={spaces.again} />;
  if (spaces.data.length === 0) return <p className={cn(muted, "text-[13px]")}>The account has no projects yet.</p>;
  return (
    <ul className={cn(card, "divide-y p-1")} aria-label="Projects">
      {spaces.data.map((s) => (
        <li key={s.space_id} className="flex items-center justify-between gap-3 px-3 py-2 text-[13px]" data-project>
          <span className="min-w-0 truncate">{s.name}</span>
          <span className={cn(muted, "shrink-0")}>
            {s.documents === undefined ? "" : `${s.documents} documents`}
            {s.awaiting_review ? ` · ${s.awaiting_review} waiting for review` : ""}
          </span>
        </li>
      ))}
    </ul>
  );
}

export function Keyboard() {
  const table = (rows: { id: string; label: string; accel: string }[]) =>
    rows.map((s) => (
      <Row key={s.id} label={s.label}>
        <kbd className="bg-muted rounded px-1.5 py-0.5 font-mono text-[12px]">{symbols(s.accel)}</kbd>
      </Row>
    ));
  return (
    <>
      <Section title="Frame">{table(FRAME_SHORTCUTS)}</Section>
      <Section title="Window layout" note="Only where the window is wide enough to arrange.">
        {table(LAYOUT_SHORTCUTS)}
      </Section>
    </>
  );
}

export function Features({ onEdit }: PaneProps) {
  const states = useFeatures();
  const undo = useUndo();
  const anyDown = FEATURES.some((f) => isDown(entryOf(states, f.id)));
  return (
    <Section
      title="Pages"
      note="Kept in this browser: a page in maintenance shows why, and when it is back. It does not stop anything on a server. The wrench at the top brings any of them back."
    >
      {FEATURES.map((f) => {
        const e = entryOf(states, f.id);
        const down = isDown(e);
        return (
          <Row key={f.id} label={f.name}>
            <span className={down ? "text-foreground" : muted} data-feature-state={down ? "maintenance" : "online"}>
              {down ? (e.reason !== "" ? `In maintenance: ${e.reason}` : "In maintenance") : "Online"}
            </span>
            {down ? (
              <button type="button" className={btn} onClick={() => backOnline(f.id)}>
                Bring back
              </button>
            ) : (
              <button type="button" className={btn} onClick={() => onEdit(f.id)}>
                Put in maintenance
              </button>
            )}
          </Row>
        );
      })}
      <div className="flex items-center gap-2 pt-2">
        <button type="button" className={btn} disabled={!anyDown} onClick={allOnline}>
          Bring everything online
        </button>
        {undo ? (
          <button type="button" className={btn} onClick={undoLast}>
            Undo: {undo.label}
          </button>
        ) : null}
      </div>
    </Section>
  );
}

const has = (k: string) => {
  try {
    return localStorage.getItem(k) !== null;
  } catch {
    return false;
  }
};

/** Takes the keys out of this browser and loads the page again, so every store starts from its default. */
function forget(keys: string[]) {
  try {
    for (const k of keys) localStorage.removeItem(k);
  } catch {
    // Storage refused: nothing was kept to begin with.
  }
  window.location.reload();
}

export function Privacy() {
  const now = heldNow(has);
  return (
    <Section
      title="Kept in this browser"
      note="Only the choices below, on this computer. Your credential is never one of them: the local server holds it, and signing out removes it."
    >
      {HELD.map((h) => (
        <Row key={h.key} label={h.label}>
          <span className={muted} data-held={now.includes(h.key) ? "yes" : "no"}>
            {now.includes(h.key) ? "Stored" : "Default"}
          </span>
          <button type="button" className={btn} disabled={!now.includes(h.key)} onClick={() => forget([h.key])}>
            Clear
          </button>
        </Row>
      ))}
      <div className="pt-2">
        <button
          type="button"
          className={btn}
          disabled={now.length === 0}
          onClick={() => forget(HELD.map((h) => h.key))}
        >
          Clear everything
        </button>
      </div>
    </Section>
  );
}

export function Developer({ session, listing }: PaneProps) {
  const reads = homeReads("{account}");
  const team = settingsReads("{account}");
  const paths = [
    reads.me.path,
    reads.reviews.path,
    reads.spaces.path,
    reads.monitors.path,
    reads.incidents.path,
    team.members.path,
  ];
  return (
    <>
      <Section title="This page">
        <Row label="Served from">{window.location.origin}</Row>
        <Row label="Sign-in">
          {session === null
            ? "checking"
            : session.state === "signed-in"
              ? `signed in (${session.profile})`
              : session.state}
        </Row>
        <Row label="Extension listing">
          {listing === null ? "loading" : listing.state === "ok" ? `${listing.panels.length} with a panel` : "failed"}
        </Row>
      </Section>
      <Section
        title="What the pages read"
        note="Read-only calls to the API through this server, as the person who signed in."
      >
        <ul className="font-mono text-[12px]">
          {paths.map((p) => (
            <li key={p} className="border-t py-1 first:border-t-0">
              GET /api/proxy{p}
            </li>
          ))}
        </ul>
      </Section>
    </>
  );
}

export function About({ listing }: PaneProps) {
  return (
    <Section
      title="iohr ui serve"
      note="The local page of the iohr command line (RFC 0217). It runs on this computer and answers only to it."
    >
      <Row label="Address">{window.location.origin}</Row>
      <Row label="Installed panels">
        {listing === null ? "loading" : listing.state === "ok" ? String(listing.panels.length) : "unknown"}
      </Row>
      <Row label="Start it with">
        <code className="font-mono text-[12px]">iohr ui open</code>
      </Row>
    </Section>
  );
}

/** A pane that works somewhere else: it says where, and shows no control that does nothing. */
export function NotHere({ pane }: { pane: Pane }) {
  return (
    <section className={cn(card, "p-4")} data-pane-state="not-built">
      <h3 className="text-sm font-semibold">Not on this page yet</h3>
      <p className={cn(muted, "mt-1 text-[13px]")}>
        {pane.label} works in {pane.where}. This page has no version of it, and shows no control that would do nothing.
      </p>
    </section>
  );
}
