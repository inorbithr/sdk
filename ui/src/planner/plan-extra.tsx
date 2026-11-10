import { useMemo, useState } from "react";
import { parseDiff } from "../lib/diff.ts";
import { ageOf, blockedItems, DOC_KINDS, evidenceOf } from "../lib/plan-evidence.ts";
import { ITEM_KINDS, type Item, type Plan, STATUSES } from "../lib/planner.ts";
import { matchesSearch } from "../lib/plans-overview.ts";
import { plannerHref } from "../lib/planner-route.ts";
import type { StageData } from "./planner-data.ts";
import { card, Chip, field } from "./ui.tsx";

const isWork = (i: Item) => i.kind !== "goal" && i.kind !== "note" && i.status !== "dropped";

/** Pull-request preview and evidence: what each item's patch changes, and what the gate says is missing. "On preview" and "verified" are not claimed. */
export function PreviewView({ planId, plan, st }: { planId: string; plan: Plan; st: StageData }) {
  const blocked = useMemo(() => blockedItems(plan, st.prs, new Map(), st.stages), [plan, st]);
  const withPatch = plan.items.filter((i) => i.preview?.patch);
  return (
    <section aria-label="Preview and evidence" className="flex flex-col gap-3">
      {blocked.length > 0 && (
        <ul className={`${card} p-3 text-[13px]`} aria-label="Blocked">
          {blocked.map(({ item, blocked: why }) => (
            <li key={item.id}>
              <a className="underline" href={plannerHref({ planId, itemId: item.id })}>{item.title}</a> {why}
            </li>
          ))}
        </ul>
      )}
      {plan.items.filter(isWork).map((i) => {
        const e = evidenceOf(plan, i, st.prs, new Map(), st.stages);
        return (
          <article key={i.id} className={`${card} p-3 text-[13px]`}>
            <h3 className="font-medium">{i.title}</h3>
            <p className="text-muted-foreground text-[12px]">
              {DOC_KINDS.map((k) => `${k.toUpperCase()} ${e.docs[k] ? (e.docs[k]?.ref ? "linked" : "not linked") : "none"}`).join(" · ")}
              {e.prs.length ? ` · ${e.prs.map((p) => `${p.ref} ${p.state}`).join(", ")}` : " · no pull request"}
              {e.diagram.present ? ` · diagram ${ageOf(e.diagram.updatedAt)}${e.diagram.stale ? " (older than head)" : ""}` : " · no diagram"}
            </p>
            {e.missing.length > 0 && <p className="text-amber-700 dark:text-amber-400">{e.fromServer ? "The gate says" : "Missing"}: {e.missing.join("; ")}</p>}
          </article>
        );
      })}
      {withPatch.map((i) => (
        <article key={`p-${i.id}`} className={`${card} p-3`}>
          <h3 className="text-[13px] font-medium">{i.title}: {i.preview?.repo} {i.preview?.branch}</h3>
          {parseDiff(i.preview?.patch ?? "").map((f) => (
            <details key={f.path} className="text-[12px]">
              <summary>
                {f.path} <span className="text-green-700 dark:text-green-400">+{f.additions}</span> <span className="text-red-700 dark:text-red-400">-{f.deletions}</span>
              </summary>
              <pre className="overflow-x-auto text-[11px]">
                {f.lines.map((l, n) => (
                  // biome-ignore lint/suspicious/noArrayIndexKey: diff lines have no id
                  <div key={n} className={l.t === "add" ? "bg-green-500/10" : l.t === "del" ? "bg-red-500/10" : undefined}>
                    {l.text}
                  </div>
                ))}
              </pre>
            </details>
          ))}
        </article>
      ))}
    </section>
  );
}

/** Decisions: the PRD, RFC and ADR each item carries, linked to the Decisions page; plain text, no Markdown rendering of their bodies here. */
export function DecisionsView({ plan }: { plan: Plan }) {
  const rows = plan.items.filter(isWork).flatMap((i) => {
    const docs = evidenceOf(plan, i).docs;
    return DOC_KINDS.flatMap((k) => (docs[k]?.ref ? [{ item: i, kind: k, ref: docs[k].ref }] : []));
  });
  return (
    <section aria-label="Decisions" className={`${card} p-3 text-[13px]`}>
      <a className="underline" href="/decisions/~waiting">What waits on you in Decisions</a>
      {rows.length === 0 ? <p className="text-muted-foreground mt-2">No item links a document yet.</p> : null}
      <ul className="mt-2 flex flex-col gap-1">
        {rows.map(({ item, kind, ref }) => (
          <li key={`${item.id}${kind}`}>
            <Chip>{kind.toUpperCase()}</Chip>{" "}
            <a className="underline" href={`/decisions/${encodeURIComponent(ref.space_id)}/${encodeURIComponent(ref.document_id)}`}>{item.title}</a>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** Explore: search and filter the items of this plan. */
export function ExploreView({ planId, plan, st }: { planId: string; plan: Plan; st: StageData }) {
  const [q, setQ] = useState("");
  const [kind, setKind] = useState("");
  const [status, setStatus] = useState("");
  const rows = plan.items.filter(
    (i) => (kind === "" || i.kind === kind) && (status === "" || i.status === status) && matchesSearch(`${i.title} ${i.id} ${i.by} ${i.body}`.toLowerCase(), q),
  );
  return (
    <section aria-label="Explore" className="flex flex-col gap-3">
      <div className="flex flex-wrap gap-2">
        <input aria-label="Search items" placeholder="Search items" className={`${field} w-56`} value={q} onChange={(e) => setQ(e.target.value)} />
        <select aria-label="Kind" className={`${field} w-auto`} value={kind} onChange={(e) => setKind(e.target.value)}>
          <option value="">Any kind</option>
          {ITEM_KINDS.map((k) => <option key={k}>{k}</option>)}
        </select>
        <select aria-label="Status" className={`${field} w-auto`} value={status} onChange={(e) => setStatus(e.target.value)}>
          <option value="">Any status</option>
          {STATUSES.map((k) => <option key={k}>{k}</option>)}
        </select>
      </div>
      <ul className={`${card} p-3 text-[13px]`}>
        {rows.length === 0 ? <li className="text-muted-foreground">No item matches.</li> : null}
        {rows.map((i) => (
          <li key={i.id} className="py-0.5">
            <a className="underline" href={plannerHref({ planId, itemId: i.id })}>{i.title}</a>{" "}
            <span className="text-muted-foreground text-[12px]">{i.kind} · {st.stages.get(i.id)?.label ?? i.status}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}
