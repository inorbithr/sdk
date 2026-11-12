// The account name, the account list and the switcher menu (RFC 0217.1, slice 1): pure rules, no React.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { flatten, isAction, isHeading } from "../src/frame/context-menu-core.ts";
import {
  accountMenu,
  accountName,
  accountRows,
  type MeWire,
  pickActive,
  planLabel,
  roleLabel,
  withAccount,
} from "../src/lib/accounts-core.ts";
import type { SessionState } from "../src/lib/api.ts";

const me: MeWire = {
  account: { id: "acc-me", kind: "personal", name: "Nevio", plan: "personal", role: "owner" },
  teams: [
    { id: "acc-z", kind: "team", name: "Zeta Labs", slug: "zeta", plan: "team", role: "member" },
    { id: "acc-a", kind: "team", name: "Acme", slug: "acme", plan: "free", role: "admin" },
  ],
  preferences: { open_account_id: "acc-a" },
};
const noop = () => {};
const act = { switchTo: noop, signIn: noop, signOut: noop };

test("the account list is the person's own first, then the teams by name, each with plan and role", () => {
  const rows = accountRows(me);
  assert.deepEqual(
    rows.map((r) => [r.name, r.kind, r.plan, r.role]),
    [
      ["Nevio", "personal", "personal", "owner"],
      ["Acme", "team", "free", "admin"],
      ["Zeta Labs", "team", "team", "member"],
    ],
  );
});

test("a row with no id is dropped, and a missing name falls back to words, never the id", () => {
  const rows = accountRows({
    account: { id: "acc-1", kind: "personal", name: "" },
    teams: [
      { kind: "team", name: "No id" },
      { id: "acc-2", kind: "team", name: "", slug: "ops" },
      { id: "acc-3", kind: "team" },
    ],
  });
  assert.deepEqual(
    rows.map((r) => r.name),
    ["Personal account", "ops", "Team"],
  );
  assert.ok(rows.every((r) => !r.name.includes("acc-")));
  assert.deepEqual(accountRows(null), []);
});

test("the active account is this tab's pick while it still exists, else the one to open, else the token's, else the first", () => {
  const rows = accountRows(me);
  assert.equal(pickActive(rows, "acc-z", "acc-a", "acc-me"), "acc-z");
  assert.equal(pickActive(rows, "gone", "acc-a", "acc-me"), "acc-a");
  assert.equal(pickActive(rows, null, "gone", "acc-z"), "acc-z");
  assert.equal(pickActive(rows, null, null, "gone"), "acc-me");
  assert.equal(pickActive([], "x", "y", "z"), null);
});

test("the header says the account's name; while the list is not there yet it says so in words, never an id", () => {
  const rows = accountRows(me);
  assert.equal(accountName(rows, "acc-a"), "Acme");
  assert.equal(accountName(rows, "unknown-id"), "Your account");
  assert.equal(accountName([], null), "Your account");
});

test("plan and role read as words", () => {
  assert.equal(planLabel("free"), "Free");
  assert.equal(planLabel(""), "Free");
  assert.equal(roleLabel("owner"), "Owner");
  assert.equal(roleLabel(""), "Member");
});

test("withAccount puts the active account in a signed-in session and leaves the others as they are", () => {
  const inn: SessionState = {
    state: "signed-in",
    profile: "production",
    kind: "person",
    account: "acc-me",
    subject: "sub",
    scopes: [],
    login: "idle",
  };
  const next = withAccount(inn, "acc-a");
  assert.equal(next?.state === "signed-in" ? next.account : "", "acc-a");
  assert.equal(withAccount(inn, null), inn);
  const out: SessionState = { state: "signed-out", login: "idle", reason: null };
  assert.equal(withAccount(out, "acc-a"), out);
  assert.equal(withAccount(null, "acc-a"), null);
});

const projects = [
  { id: "lab_1", name: "inorbit.hr" },
  { id: "lab_2", name: "Docs" },
];

test("the menu lists accounts with a check on the current one, then projects with the environment, then sign out", () => {
  const entries = accountMenu({
    rows: accountRows(me),
    active: "acc-a",
    projects,
    profile: "production",
    signedIn: true,
    ...act,
  });
  const heads = entries.filter(isHeading).map((h) => h.heading);
  assert.deepEqual(heads, ["Accounts", "Projects"]);
  const acts = entries.filter(isAction);
  const accounts = acts.filter((a) => a.id.startsWith("account:"));
  assert.deepEqual(
    accounts.map((a) => [a.label, a.hint, a.checked]),
    [
      ["Nevio", "Personal · Owner", false],
      ["Acme", "Free · Admin", true],
      ["Zeta Labs", "Team · Member", false],
    ],
  );
  const project = acts.find((a) => a.id === "project:lab_2");
  assert.equal(project?.label, "Docs");
  const envs = project?.items?.filter(isAction) ?? [];
  assert.deepEqual(
    envs.map((e) => [e.label, e.checked]),
    [["production", true]],
  );
  assert.ok(acts.some((a) => a.id === "session:out"));
  assert.ok(!acts.some((a) => a.id === "session:in"));
});

test("choosing another account switches to it; choosing the current one does nothing", () => {
  const got: string[] = [];
  const entries = accountMenu({
    rows: accountRows(me),
    active: "acc-a",
    projects: [],
    profile: "production",
    signedIn: true,
    ...act,
    switchTo: (id) => got.push(id),
  });
  const by = (id: string) => flatten(entries).find((f) => f.action.id === id)?.action;
  by("account:acc-z")?.run?.();
  assert.deepEqual(got, ["acc-z"]);
  by("account:acc-a")?.run?.();
  assert.deepEqual(got, ["acc-z"]);
});

test("New project and Add environment are there, off, and say why", () => {
  const entries = accountMenu({
    rows: accountRows(me),
    active: "acc-a",
    projects,
    profile: "production",
    signedIn: true,
    ...act,
  });
  const by = (id: string) => flatten(entries).find((f) => f.action.id === id)?.action;
  for (const id of ["project:new", "project:env"]) {
    const a = by(id);
    assert.equal(typeof a?.disabled, "string", id);
    assert.ok(String(a?.disabled).length > 10);
  }
  assert.equal(by("project:new")?.label, "New project…");
  assert.equal(by("project:env")?.label, "Add environment…");
});

test("signed out, the menu offers Sign in and nothing that needs the account", () => {
  const entries = accountMenu({ rows: [], active: null, projects: [], profile: "", signedIn: false, ...act });
  const acts = flatten(entries).map((f) => f.action.id);
  assert.deepEqual(acts, ["session:in"]);
});

test("signed in with the list not loaded, the menu still has sign out and says nothing is listed yet", () => {
  const entries = accountMenu({ rows: [], active: null, projects: [], profile: "production", signedIn: true, ...act });
  const acts = flatten(entries).map((f) => f.action);
  assert.ok(acts.some((a) => a.id === "session:out"));
  assert.ok(!acts.some((a) => a.id.startsWith("account:")));
});

test("no surface renders the session's account id or subject: the header, the rail and Settings say the name", () => {
  for (const f of ["session-control.tsx", "settings-panes.tsx"]) {
    const source = readFileSync(new URL(`../src/${f}`, import.meta.url), "utf8");
    assert.doesNotMatch(source, /\{session\.subject\}|\{session\.account\}/, `${f} shows a raw id`);
    assert.doesNotMatch(source, /session\.account === "" \? session\.subject/, `${f} falls back to the subject`);
  }
});
