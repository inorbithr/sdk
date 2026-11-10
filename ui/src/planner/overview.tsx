import { useMemo, useState } from "react";
import {
  FILTERS,
  filterCounts,
  keepRow,
  matchesSearch,
  type OverviewFilter,
  type OverviewPlan,
  prUrl,
  rowOf,
  sortPlans,
  teamsOf,
  totalsOf,
} from "../lib/plans-overview.ts";
import { plannerHref } from "../lib/planner-route.ts";
import { ageOf } from "../lib/plan-evidence.ts";
import { card, Chip, field, StubChip } from "./ui.tsx";

const WORD: Record<string, string> = {
  not_started: "Not started",
  running: "Running",
  waiting_on_person: "Waiting on you",
  waiting_on_pr: "Waiting on a PR",
  done: "Done",
  failed: "Failed",
};
const TONE = { done: "good", running: "good", failed: "bad", waiting_on_person: "warn", waiting_on_pr: "warn", not_started: "idle" } as const;

/** The plans overview: every plan, its progress word, what it waits on, filters, search and totals (RFC 0166). */
export function Overview({ plans, ready }: { plans: OverviewPlan[]; ready: boolean }) {
  const [filter, setFilter] = useState<OverviewFilter>("all");
  const [team, setTeam] = useState("");
  const [q, setQ] = useState("");
  const counts = useMemo(() => filterCounts(plans, team), [plans, team]);
  const totals = useMemo(() => totalsOf(plans), [plans]);
  const rows = useMemo(
    () => sortPlans(plans.filter((p) => keepRow(p, filter, team))).map((p) => rowOf(p)).filter((r) => matchesSearch(r.search, q)),
    [plans, filter, team, q],
  );
  const teams = teamsOf(plans);
  return (
    <section aria-label="Plans overview" className="flex flex-col gap-3">
      <p className="text-muted-foreground text-[12px]">
        {totals.plans} plans, {totals.running_jobs} running jobs, {totals.waiting_on_me} waiting on you. Read from the coordinator's jobs and asks; nothing here is typed in.
      </p>
      <div className="flex flex-wrap items-center gap-2">
        {FILTERS.map((f) => (
          <button
            key={f.id}
            type="button"
            aria-pressed={filter === f.id}
            onClick={() => setFilter(f.id)}
            className={`rounded-full border px-3 py-1 text-[12px] ${filter === f.id ? "bg-accent" : "hover:bg-accent"}`}
          >
            {f.label} {counts[f.id]}
          </button>
        ))}
        {teams.length > 0 && (
          <select aria-label="Team" className={`${field} w-auto`} value={team} onChange={(e) => setTeam(e.target.value)}>
            <option value="">All teams</option>
            {teams.map((t) => (
              <option key={t}>{t}</option>
            ))}
          </select>
        )}
        <input aria-label="Search plans" placeholder="Search plans" className={`${field} w-56`} value={q} onChange={(e) => setQ(e.target.value)} />
      </div>
      {!ready && plans.length === 0 ? <p className="text-muted-foreground text-[13px]">Waiting for the first snapshot…</p> : null}
      {ready && rows.length === 0 ? <p className="text-muted-foreground text-[13px]">No plan matches.</p> : null}
      <ul className="flex flex-col gap-2">
        {rows.map((p) => (
          <li key={p.plan_id} className={`${card} p-3`}>
            <div className="flex flex-wrap items-center gap-2">
              <a className="text-[14px] font-medium hover:underline" href={plannerHref({ planId: p.plan_id })}>
                {p.title || p.plan_id}
              </a>
              <Chip tone={TONE[p.progress]}>{WORD[p.progress] ?? p.progress}</Chip>
              {p.state !== "active" ? <Chip>{p.state}</Chip> : null}
              {p.stub ? <StubChip /> : null}
              {p.team ? <span className="text-muted-foreground text-[11px]">{p.team}</span> : null}
            </div>
            <p className="text-muted-foreground mt-1 text-[12px]">
              {p.counts.done} done, {p.counts.running} running, {p.counts.waiting_on_person + p.counts.waiting_on_pr} waiting, {p.counts.failed} failed, {p.counts.not_started} not started
              {p.waiting_on.kind !== "none" ? ` · waiting on ${p.waiting_on.kind} ${p.waiting_on.ref}`.trimEnd() : ""}
              {p.release_stage === "none" ? "" : ` · release ${p.release_stage.replace("_", " ")}`}
              {p.updated_at ? ` · ${ageOf(p.updated_at)}` : ""}
            </p>
            {p.prs.length > 0 && (
              <p className="mt-1 flex flex-wrap gap-2 text-[12px]">
                {p.prs.map((r) => (
                  <a key={r.ref} className="underline" href={prUrl(r.ref) || undefined} target="_blank" rel="noreferrer">
                    {r.ref} {r.state}
                    {r.stub ? " (stub)" : ""}
                  </a>
                ))}
              </p>
            )}
            {p.stuck.length > 0 && <p className="text-[12px] text-amber-700 dark:text-amber-400">Stuck: {p.stuck.join("; ")}</p>}
          </li>
        ))}
      </ul>
    </section>
  );
}
