// Team management rules and the calls behind them (RFC 0217.1, slice 2). Pure: no React, no DOM, so node tests it.
// The rules are the console's (`ui/console` Team pages) and the API's own (accounts.proto: UpdateMember, RemoveMember,
// CreateInvitation): an owner or admin manages; only the owner touches an admin; nobody changes or removes the owner
// here (that is a transfer, in the console); anyone may leave. Every call is a route the console already uses.
// Nothing here shows an id: people are named by name or e-mail.

export type Role = "owner" | "admin" | "member";
export type Person = { user_id?: string; email?: string; name?: string; role?: string };
export type Invite = {
  id?: string;
  email?: string;
  role?: string;
  status?: string;
  expires_at?: string;
  invited_by_name?: string;
};

/** The roles a person can be given here; the owner's role changes by transfer, in the console. */
export const GRANTABLE: { id: Exclude<Role, "owner">; label: string; hint: string }[] = [
  { id: "member", label: "Member", hint: "Works in the team's projects; cannot manage people." },
  { id: "admin", label: "Admin", hint: "Also invites, removes and changes members." },
];

const asRole = (r: string | undefined): Role => (r === "owner" || r === "admin" ? r : "member");

export const manages = (myRole: string | undefined): boolean => myRole === "owner" || myRole === "admin";

export type MemberRules = { changeRole: boolean; remove: boolean; leave: boolean };

/** What the signed-in person may do to one row, as the console decides it. */
export function memberRules(m: Person, selfId: string, myRole: string | undefined): MemberRules {
  const theirs = asRole(m.role);
  const you = m.user_id !== undefined && m.user_id === selfId;
  const reach = theirs !== "owner" && !you && (myRole === "owner" || (manages(myRole) && theirs === "member"));
  return { changeRole: reach, remove: reach, leave: you && theirs !== "owner" };
}

/** The name a person is shown by: their name, else their address, else words. */
export const personName = (p: { name?: string; email?: string }): string => p.name || p.email || "Someone";

export const roleWord = (r: string | undefined): string => {
  const role = asRole(r);
  return role.charAt(0).toUpperCase() + role.slice(1);
};

/** True for one plain address; the API checks it again. */
export const emailOk = (s: string): boolean => /^[^\s@]+@[^\s@.]+(\.[^\s@.]+)+$/.test(s.trim());

/** True for a name the API will take: not empty, at most 80 characters once trimmed. */
export const teamNameOk = (s: string): boolean => s.trim().length > 0 && s.trim().length <= 80;

export type Call = { method: "POST" | "PATCH" | "PUT" | "DELETE"; path: string; body?: Record<string, unknown> };
const enc = encodeURIComponent;
const org = (id: string) => `/v1/accounts/orgs/${enc(id)}`;

export const changeRole = (orgId: string, userId: string, role: string): Call => ({
  method: "PATCH",
  path: `${org(orgId)}/members/${enc(userId)}`,
  body: { role },
});
export const removeMember = (orgId: string, userId: string): Call => ({
  method: "DELETE",
  path: `${org(orgId)}/members/${enc(userId)}`,
});
export const invite = (orgId: string, email: string, role: string): Call => ({
  method: "POST",
  path: `${org(orgId)}/invitations`,
  body: { email: email.trim(), role },
});
export const resendInvite = (orgId: string, invitationId: string): Call => ({
  method: "POST",
  path: `${org(orgId)}/invitations/${enc(invitationId)}/resend`,
  body: {},
});
export const revokeInvite = (orgId: string, invitationId: string): Call => ({
  method: "DELETE",
  path: `${org(orgId)}/invitations/${enc(invitationId)}`,
});
export const renameTeam = (orgId: string, name: string): Call => ({
  method: "PATCH",
  path: org(orgId),
  body: { name: name.trim() },
});
export const createTeam = (name: string): Call => ({
  method: "POST",
  path: "/v1/accounts/orgs",
  body: { name: name.trim() },
});
/** Remembers the account this person last used, so the next sign-in opens it (as the console does). */
export const rememberAccount = (accountId: string): Call => ({
  method: "PUT",
  path: "/v1/accounts/me/preferences/last",
  body: { account_id: accountId },
});
export const defaultAccount = (accountId: string): Call => ({
  method: "PUT",
  path: "/v1/accounts/me/preferences/default",
  body: { account_id: accountId },
});

/** Open invitations first (the ones to act on), then the rest, newest address order kept. */
export const openInvites = (list: Invite[]): Invite[] =>
  [...list].sort((a, b) => Number(isOpen(a) === false) - Number(isOpen(b) === false));
export const isOpen = (i: Invite): boolean => (i.status ?? "pending") === "pending" || i.status === "open";

/** A write's refusal in plain words: what happened, then what to do. */
export function failure(status: number, reason: string, what: string): string {
  if (status === 0) return `Could not ${what}: ${reason}`;
  if (status === 403) return `You may not ${what}. Ask an owner or admin of this team.`;
  if (status === 409) return `Could not ${what}: it changed meanwhile. Reload the list and try again.`;
  if (status === 429) return `Could not ${what}: too many tries. Wait a moment and try again.`;
  return `Could not ${what}: ${reason}`;
}
