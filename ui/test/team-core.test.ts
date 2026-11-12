// Team management (RFC 0217.1, slice 2): who may do what, and the exact calls Settings makes. Pure, no React.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import {
  changeRole,
  createTeam,
  defaultAccount,
  emailOk,
  failure,
  GRANTABLE,
  invite,
  isOpen,
  manages,
  memberRules,
  offlineWhy,
  openInvites,
  personName,
  rememberAccount,
  removeMember,
  renameTeam,
  resendInvite,
  revokeInvite,
  roleWord,
  teamNameOk,
} from "../src/lib/team-core.ts";
import { apiSend } from "../src/lib/team-send.ts";

const owner = { user_id: "u-o", role: "owner" };
const admin = { user_id: "u-a", role: "admin" };
const member = { user_id: "u-m", role: "member" };

test("an owner changes and removes admins and members, never the owner row", () => {
  assert.deepEqual(memberRules(admin, "u-o", "owner"), { changeRole: true, remove: true, leave: false });
  assert.deepEqual(memberRules(member, "u-o", "owner"), { changeRole: true, remove: true, leave: false });
  assert.deepEqual(memberRules(owner, "u-a", "owner"), { changeRole: false, remove: false, leave: false });
});

test("an admin touches members only, never another admin or the owner", () => {
  assert.equal(memberRules(member, "u-a", "admin").changeRole, true);
  assert.equal(memberRules(member, "u-a", "admin").remove, true);
  assert.equal(memberRules({ user_id: "u-b", role: "admin" }, "u-a", "admin").changeRole, false);
  assert.equal(memberRules(owner, "u-a", "admin").remove, false);
});

test("a member manages nobody", () => {
  for (const row of [owner, admin, { user_id: "u-n", role: "member" }]) {
    const r = memberRules(row, "u-m", "member");
    assert.equal(r.changeRole, false);
    assert.equal(r.remove, false);
  }
});

test("anyone but the owner may leave, and nobody changes their own role or removes themselves", () => {
  assert.deepEqual(memberRules(member, "u-m", "member"), { changeRole: false, remove: false, leave: true });
  assert.deepEqual(memberRules(admin, "u-a", "admin"), { changeRole: false, remove: false, leave: true });
  assert.equal(memberRules(owner, "u-o", "owner").leave, false);
});

test("a row with no id is never you, and an unknown role counts as member", () => {
  assert.equal(memberRules({ role: "member" }, "", "owner").leave, false);
  assert.equal(memberRules({ user_id: "x", role: "weird" }, "u-o", "owner").changeRole, true);
  assert.equal(manages(undefined), false);
  assert.equal(manages("admin"), true);
});

test("only member and admin can be granted here", () => {
  assert.deepEqual(
    GRANTABLE.map((r) => r.id),
    ["member", "admin"],
  );
});

test("people are named by name, else address, else words; roles read as words", () => {
  assert.equal(personName({ name: "Ana", email: "a@x.io" }), "Ana");
  assert.equal(personName({ email: "a@x.io" }), "a@x.io");
  assert.equal(personName({}), "Someone");
  assert.equal(roleWord("owner"), "Owner");
  assert.equal(roleWord(undefined), "Member");
});

test("the form checks: one plain address, a team name of 1 to 80 characters", () => {
  assert.equal(emailOk(" ana@example.com "), true);
  for (const bad of ["", "ana", "ana@", "@x.io", "a b@x.io", "ana@x", "ana@x..io"])
    assert.equal(emailOk(bad), false, bad);
  assert.equal(teamNameOk("Acme"), true);
  assert.equal(teamNameOk("   "), false);
  assert.equal(teamNameOk("x".repeat(80)), true);
  assert.equal(teamNameOk("x".repeat(81)), false);
});

test("each call is a route the console already uses, with encoded ids and only the fields the API reads", () => {
  assert.deepEqual(changeRole("o/1", "u 2", "admin"), {
    method: "PATCH",
    path: "/v1/accounts/orgs/o%2F1/members/u%202",
    body: { role: "admin" },
  });
  assert.deepEqual(removeMember("o1", "u2"), { method: "DELETE", path: "/v1/accounts/orgs/o1/members/u2" });
  assert.deepEqual(invite("o1", " ana@x.io ", "member"), {
    method: "POST",
    path: "/v1/accounts/orgs/o1/invitations",
    body: { email: "ana@x.io", role: "member" },
  });
  assert.deepEqual(resendInvite("o1", "i1"), {
    method: "POST",
    path: "/v1/accounts/orgs/o1/invitations/i1/resend",
    body: {},
  });
  assert.deepEqual(revokeInvite("o1", "i1"), { method: "DELETE", path: "/v1/accounts/orgs/o1/invitations/i1" });
  assert.deepEqual(renameTeam("o1", " New "), { method: "PATCH", path: "/v1/accounts/orgs/o1", body: { name: "New" } });
  assert.deepEqual(createTeam(" Ops "), { method: "POST", path: "/v1/accounts/orgs", body: { name: "Ops" } });
  assert.deepEqual(rememberAccount("o1"), {
    method: "PUT",
    path: "/v1/accounts/me/preferences/last",
    body: { account_id: "o1" },
  });
  assert.deepEqual(defaultAccount("o1"), {
    method: "PUT",
    path: "/v1/accounts/me/preferences/default",
    body: { account_id: "o1" },
  });
});

test("open invitations list first", () => {
  const list = [
    { id: "1", status: "accepted" },
    { id: "2", status: "pending" },
    { id: "3", status: "revoked" },
    { id: "4" },
  ];
  assert.deepEqual(
    openInvites(list).map((i) => i.id),
    ["2", "4", "1", "3"],
  );
  assert.equal(isOpen({ status: "expired" }), false);
});

test("a refusal says what happened and what to do", () => {
  assert.match(failure(403, "x", "remove Ana"), /You may not remove Ana\. Ask an owner or admin/);
  assert.match(failure(409, "x", "invite Ana"), /changed meanwhile/);
  assert.match(failure(429, "x", "invite Ana"), /too many tries/);
  assert.equal(
    failure(0, "The local `iohr ui serve` did not answer.", "invite Ana"),
    "Could not invite Ana: The local `iohr ui serve` did not answer.",
  );
  assert.equal(
    failure(400, "That address is not valid.", "invite Ana"),
    "Could not invite Ana: That address is not valid.",
  );
});

type Seen = { url: string; init: RequestInit };
const fake = (status: number, body: unknown, seen: Seen[] = []): typeof fetch =>
  (async (url: string, init: RequestInit) => {
    seen.push({ url, init });
    return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  }) as unknown as typeof fetch;

test("a write goes to the proxy with the start token and a JSON body, and the answer is read as a Read", async () => {
  const seen: Seen[] = [];
  const r = await apiSend("PATCH", "/v1/accounts/orgs/o1", { name: "N" }, {}, "tok", fake(200, { ok: 1 }, seen));
  assert.deepEqual(r, { state: "ok", data: { ok: 1 } });
  assert.equal(seen[0]?.url, "/api/proxy/v1/accounts/orgs/o1");
  const init = seen[0]?.init;
  assert.equal(init?.method, "PATCH");
  assert.equal(init?.body, JSON.stringify({ name: "N" }));
  const h = new Headers(init?.headers);
  assert.equal(h.get("x-iohr-token"), "tok");
  assert.equal(h.get("content-type"), "application/json");
});

test("a write with no body sends no content type; 401 is signed out; an API refusal keeps its status and words", async () => {
  const seen: Seen[] = [];
  await apiSend("DELETE", "/v1/accounts/orgs/o1/members/u1", undefined, {}, "tok", fake(200, {}, seen));
  assert.equal(new Headers(seen[0]?.init.headers).get("content-type"), null);
  assert.equal(seen[0]?.init.body, undefined);
  assert.deepEqual(await apiSend("DELETE", "/v1/x", undefined, {}, "tok", fake(401, {})), { state: "signed-out" });
  assert.deepEqual(await apiSend("POST", "/v1/accounts/orgs", {}, {}, "tok", fake(403, { error: "Not allowed." })), {
    state: "error",
    status: 403,
    reason: "Not allowed.",
  });
  const none = await apiSend("POST", "/v1/accounts/orgs", {}, {}, null, fake(200, {}));
  assert.equal(none.state, "error");
});

test("a path that is not under /v1 is refused before anything is sent", async () => {
  const seen: Seen[] = [];
  const r = await apiSend("POST", "/other", {}, {}, "tok", fake(200, {}, seen));
  assert.equal(r.state, "error");
  assert.equal(seen.length, 0);
});

test("Settings writes only through apiSend and useWrites, never a raw fetch; switching accounts remembers the choice", () => {
  const read = (f: string) => readFileSync(new URL(`../src/${f}`, import.meta.url), "utf8");
  const panes = read("settings-panes.tsx");
  assert.doesNotMatch(panes, /fetch\(/);
  assert.match(panes, /useWrites\(\)/);
  assert.match(read("lib/settings-data.ts"), /apiSend\(/);
  assert.match(read("lib/accounts-store.ts"), /rememberAccount\(/);
});

test("offline, writes are off with a plain reason; online there is none", () => {
  assert.equal(offlineWhy(true), null);
  assert.match(String(offlineWhy(false)), /cannot be reached.*changes are off.*Reading works/);
  const data = readFileSync(new URL("../src/lib/settings-data.ts", import.meta.url), "utf8");
  assert.match(data, /offlineWhy\(useOnline\(\)\)/);
  const panes = readFileSync(new URL("../src/settings-panes.tsx", import.meta.url), "utf8");
  assert.match(panes, /data-write="offline"/);
});
