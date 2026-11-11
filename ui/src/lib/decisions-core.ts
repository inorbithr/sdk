// The pure parts of the Decisions reader's review side (PRD 0029, part 2), ported from the
// console (ui/console/src/components/lab/pages/doc-review.tsx, doc-relations.tsx, checks.tsx):
// where a decision stands, the statuses a kind has, what assistants may approve, the diff
// shown in hunks, linked pull requests found in the text, and findings split by how they block.
// No React, no network: `pnpm test` runs it with node:test.

import type { Via } from "./io-via.ts";

export type ReviewRow = {
  review_id: string;
  reviewer?: string;
  requested_by?: string;
  via?: Via | null;
  decision?: string;
  version?: number;
  requested_at?: string;
  decided_at?: string;
};
export type ReviewsAnswer = { reviews?: ReviewRow[]; approvals?: number; approvals_required?: number };

export type DecisionState = {
  required: number;
  approvals: number;
  /** The latest answer of each reviewer that asked for changes on the current version. */
  changes: ReviewRow[];
  /** Why the decision can't be taken yet; "" when it can. */
  blocked: string;
};

/**
 * Where a decision stands on the current version: approvals against the rule, who asked for
 * changes (only their latest answer on this version counts), and why it is blocked.
 */
export function decisionState(currentVersion: number, reviews: ReviewsAnswer | null): DecisionState {
  const required = reviews?.approvals_required ?? 0;
  const approvals = reviews?.approvals ?? 0;
  const latest = new Map<string, ReviewRow>();
  for (const r of reviews?.reviews ?? []) {
    if (!r.reviewer || r.decision === "pending" || !r.decision || r.version !== currentVersion) continue;
    const prev = latest.get(r.reviewer);
    if (!prev || (r.decided_at ?? "") > (prev.decided_at ?? "")) latest.set(r.reviewer, r);
  }
  const changes = [...latest.values()].filter((r) => r.decision === "changes");
  const blocked = changes.length
    ? `Changes were asked on v${currentVersion}; they need an answer first.`
    : approvals < required
      ? `Needs ${required} approval${required === 1 ? "" : "s"} on v${currentVersion}; it has ${approvals}.`
      : "";
  return { required, approvals, changes, blocked };
}

/** One row per reviewer: their latest request or answer (the API lists newest first). */
export function latestPerReviewer(reviews: ReviewRow[]): ReviewRow[] {
  const seen = new Map<string, ReviewRow>();
  for (const r of reviews) if (r.reviewer && !seen.has(r.reviewer)) seen.set(r.reviewer, r);
  return [...seen.values()];
}

/** What the signed-in person can do on a review of this version. */
export function myReviewState(me: string, createdBy: string | undefined, currentVersion: number, reviews: ReviewRow[]) {
  const author = !!me && createdBy === me;
  const mine = reviews.find((r) => r.reviewer === me && r.decision === "pending");
  const answered = reviews.find(
    (r) => r.reviewer === me && r.decision && r.decision !== "pending" && r.version === currentVersion,
  );
  return { author, mine, answered, canAnswer: !author && (!!mine || !answered) };
}

// ---------------------------------------------------------------- statuses

export type Kind = "prd" | "rfc" | "adr" | "study";
const KINDS: Kind[] = ["prd", "rfc", "adr", "study"];
export const asKind = (k: string | undefined): Kind => (KINDS.includes(k as Kind) ? (k as Kind) : "rfc");

/** The statuses of each kind, as the console's menu lists them. The API refuses one that isn't a kind's. */
export const STATUS_OF: Record<Kind, readonly string[]> = {
  prd: ["draft", "in_review", "decided", "superseded"],
  rfc: ["draft", "in_review", "decided", "superseded"],
  adr: ["proposed", "accepted", "superseded"],
  study: ["running", "measured", "published"],
};
/** The status a decision sets, which needs the space's approvals. */
export const DECIDED: Record<Kind, string> = { prd: "decided", rfc: "decided", adr: "accepted", study: "published" };

/** The menu: every status of the kind but the current one (the console's "Set status"). */
export function otherStatuses(kind: string | undefined, current: string | undefined): string[] {
  return STATUS_OF[asKind(kind)].filter((s) => s !== current);
}

/** Whether the one-click "Decide" belongs here: not already decided, not superseded. */
export function canDecide(kind: string | undefined, status: string | undefined): boolean {
  return status !== DECIDED[asKind(kind)] && status !== "superseded";
}

export const statusText = (s: string) => s.replace(/_/g, " ");

/** "Ask for review" only moves into in_review for kinds that have it. */
export const hasReviewStatus = (kind: string | undefined) => STATUS_OF[asKind(kind)].includes("in_review");

// ---------------------------------------------------------------- ai_approval

/** The space's ai_approval as the settings answer it: "manual", "auto", or a mode with kinds. */
export type AiApproval = string | { mode?: string; kinds?: string[] } | null | undefined;

/** Whether assistants may approve documents of this kind (manual unless auto lists it). */
export function aiMayApprove(setting: AiApproval, kind: string | undefined): boolean {
  if (!setting) return false;
  const k = (kind ?? "").toLowerCase();
  if (typeof setting === "string") return setting === "auto";
  if (setting.mode !== "auto") return false;
  return !setting.kinds || setting.kinds.map((x) => x.toLowerCase()).includes(k);
}

/** One line for the reviews card. */
export function aiApprovalText(setting: AiApproval, kind: string | undefined): string {
  return aiMayApprove(setting, kind)
    ? "Assistants may approve this kind here; their answer is recorded on a person's behalf."
    : "Only people approve here. Assistants can't answer a review in this space.";
}

// ---------------------------------------------------------------- the diff

export type DiffLine = { kind: string; text: string };
export type Hunk = { start: number; lines: DiffLine[]; skipped: number };

/**
 * The diff as hunks: every changed line with `context` unchanged lines around it, and the
 * stretches of unchanged lines between them counted rather than shown.
 */
export function hunks(lines: DiffLine[], context = 2): Hunk[] {
  const keep = new Array<boolean>(lines.length).fill(false);
  lines.forEach((l, i) => {
    if (l.kind === "same") return;
    for (let j = Math.max(0, i - context); j <= Math.min(lines.length - 1, i + context); j++) keep[j] = true;
  });
  const out: Hunk[] = [];
  let gap = 0;
  lines.forEach((l, i) => {
    if (!keep[i]) {
      gap++;
      return;
    }
    const last = out[out.length - 1];
    if (!last || gap > 0) out.push({ start: i, lines: [], skipped: gap });
    out[out.length - 1]!.lines.push(l);
    gap = 0;
  });
  return out;
}

export function diffCounts(lines: DiffLine[]) {
  let added = 0;
  let removed = 0;
  for (const l of lines) {
    if (l.kind === "added") added++;
    else if (l.kind === "removed") removed++;
  }
  return { added, removed, unchanged: lines.length - added - removed };
}

/** The version to compare the shown one with: the one before it, or the one the reader last saw. */
export function compareBase(shown: number, lastSeen?: number): number {
  if (shown <= 1) return 0;
  return lastSeen && lastSeen < shown ? lastSeen : shown - 1;
}

// ---------------------------------------------------------------- linked pull requests

export type PrRef = { repo: string; number: number; url: string };

const PR_URL = /https:\/\/github\.com\/([A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+)\/pull\/(\d{1,7})(?![\d])/g;

/**
 * The pull requests a document links to, by their GitHub URL, once each, in the order they
 * appear. With `only`, just those repositories (the ones the app is configured for).
 */
export function prRefs(text: string, only?: readonly string[]): PrRef[] {
  const seen = new Set<string>();
  const out: PrRef[] = [];
  for (const m of text.matchAll(PR_URL)) {
    const repo = m[1]!;
    const number = Number(m[2]);
    const key = `${repo}#${number}`;
    if (seen.has(key) || (only && !only.includes(repo))) continue;
    seen.add(key);
    out.push({ repo, number, url: `https://github.com/${repo}/pull/${number}` });
  }
  return out;
}

export type PrState = {
  state: string;
  draft: boolean;
  review: string;
  passed: number;
  failed: number;
  pending: number;
};

/** One word and a tone for a pull request's live state. */
export function prLabel(p: PrState): { text: string; tone: "good" | "warn" | "bad" | "idle" } {
  if (p.state === "MERGED") return { text: "merged", tone: "good" };
  if (p.state === "CLOSED") return { text: "closed", tone: "idle" };
  if (p.draft) return { text: "draft", tone: "idle" };
  if (p.failed > 0) return { text: `${p.failed} check${p.failed === 1 ? "" : "s"} failing`, tone: "bad" };
  if (p.review === "CHANGES_REQUESTED") return { text: "changes requested", tone: "bad" };
  if (p.pending > 0) return { text: "checks running", tone: "warn" };
  if (p.review === "APPROVED") return { text: "approved", tone: "good" };
  return { text: "open", tone: "warn" };
}

// ---------------------------------------------------------------- checks (findings)

export type Finding = { line?: number | null; rule: string; message: string };

/** A redaction finding's message starts with its own rule in brackets; it blocks publishing. */
export const isRedaction = (f: Finding) => f.message.startsWith(`[${f.rule}] `);

export function splitFindings(findings: readonly Finding[]): { redaction: Finding[]; form: Finding[] } {
  const redaction: Finding[] = [];
  const form: Finding[] = [];
  for (const f of findings) (isRedaction(f) ? redaction : form).push(f);
  return { redaction, form };
}

/** The one-line summary: "No findings", or how many are redaction and how many form. */
export function checksLabel(findings: readonly Finding[]): { text: string; tone: "good" | "warn" | "bad" } {
  if (findings.length === 0) return { text: "No findings", tone: "good" };
  const { redaction, form } = splitFindings(findings);
  const text = [redaction.length ? `${redaction.length} redaction` : "", form.length ? `${form.length} form` : ""]
    .filter(Boolean)
    .join(" · ");
  return { text, tone: redaction.length ? "bad" : "warn" };
}

/** A finding's message without the rule prefix the build's log line carries. */
export const readable = (f: Finding) => f.message.replace(/^\[[a-z0-9-]+\]\s*/, "");

// ---------------------------------------------------------------- timeline

export type TimelineEntry = {
  document_id: string;
  kind?: string;
  display_number?: string;
  title?: string;
  status?: string;
  date: string;
  text?: string;
};

/** Entries grouped by month (YYYY-MM), in the order given (newest first). */
export function byMonth(entries: TimelineEntry[]): { month: string; lines: TimelineEntry[] }[] {
  const out: { month: string; lines: TimelineEntry[] }[] = [];
  for (const e of entries) {
    const month = e.date.slice(0, 7);
    if (out[out.length - 1]?.month !== month) out.push({ month, lines: [] });
    out[out.length - 1]!.lines.push(e);
  }
  return out;
}

// ---------------------------------------------------------------- what is wanted of the person

export type WaitKind = "review" | "decide" | "question";
export type AskDoc = {
  kind: string;
  title: string;
  status?: string;
  current_version?: number;
  display_number?: string;
  number?: number;
};

/** One plain sentence for what a waiting item asks of the person. `who` is the asker's name, `when` a relative time ("3 days ago"). */
export function waitSentence(
  it: { kind: string; text?: string; unblocks?: string },
  o: { who?: string; version: number; when?: string; blocked: string; required: number; approvals: number },
): string {
  const when = o.when ? ` · ${o.when}` : "";
  if (it.kind === "question")
    return `Question${o.who ? ` from ${o.who}` : ""}: ${it.text ?? ""}${it.unblocks ? ` · unblocks ${it.unblocks}` : ""}${when}`;
  if (it.kind === "decide") {
    if (o.blocked)
      return `You decide this: it needs ${o.required} approval${o.required === 1 ? "" : "s"} on v${o.version} and has ${o.approvals}.`;
    return `You decide this: it has its ${o.required} approval${o.required === 1 ? "" : "s"} on v${o.version}, so you can take the decision.`;
  }
  return `${o.who ? `${o.who} asked you` : "You were asked"} to review v${o.version}${when}`;
}

/** The questions worth offering for a document in this state, three to five, the first being the one about what is asked. */
export function suggestedQuestions(
  d: { kind: string; status?: string; current_version?: number },
  o: { waiting: string[]; blocked: string },
): string[] {
  const out: string[] = [];
  out.push(o.waiting.length ? "Summarise what I'm being asked to decide" : "Summarise this document in five lines");
  if ((d.current_version ?? 0) > 1) out.push("What changed since the last version?");
  out.push(
    d.status === "decided" || d.status === "accepted" || d.status === "published"
      ? "What are the risks of this standing as it is?"
      : "What are the risks if I approve?",
  );
  out.push("Who and what is blocked by this?");
  if (out.length < 5) out.push("Which earlier decisions does this conflict with?");
  return out.slice(0, 5);
}

/** The buttons the Ask AI card offers beyond the band's own: reviewers who were not asked can still answer; anyone but the author can ask the author. */
export function suggestedActions(o: {
  inBand: Set<string>;
  canAnswer: boolean;
  author: boolean;
  canDecide: boolean;
  blocked: string;
}): string[] {
  const out: string[] = [];
  if (o.canAnswer && !o.author && !o.inBand.has("review")) out.push("approve", "changes");
  if (o.canDecide && !o.blocked && !o.inBand.has("decide")) out.push("decide");
  if (!o.author) out.push("ask-author");
  return out;
}

/** The context chip and the line a message to the Coordinator starts with. */
export function docContext(
  d: AskDoc,
  label: string,
  waiting: string[],
): { chip: string; wrap: (text: string) => string } {
  const facts = [
    `${label} "${d.title}"`,
    d.status ? `status ${d.status.replace(/_/g, " ")}` : "",
    d.current_version ? `version ${d.current_version}` : "",
    waiting.length ? `waiting on me: ${waiting.join("; ")}` : "",
  ]
    .filter(Boolean)
    .join(", ");
  return { chip: `${label} · ${d.title}`, wrap: (text) => `About the Decisions document ${facts}.\n\n${text}` };
}
