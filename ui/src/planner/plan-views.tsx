import { useMemo } from "react";
import { cn } from "../frame/cn.ts";
import { itemJobs } from "../lib/job-controls.ts";
import { ageOf } from "../lib/plan-evidence.ts";
import { stageLine } from "../lib/plan-stage-core.ts";
import { countBy, type Item, liveItems, needsYou, outline, type Plan } from "../lib/planner.ts";
import { decidablePrs, duration, progressOf, STAGE_LABEL, STAGES } from "../lib/planner-progress.ts";
import { plannerHref } from "../lib/planner-route.ts";
import { planScene } from "../lib/planner-scene.ts";
import { prUrl } from "../lib/plans-overview.ts";
import { JobPanel } from "./job-panel.tsx";
import type { StageData } from "./planner-data.ts";
import { Chip, card } from "./ui.tsx";

const tone = (s: string) =>
  (s === "done" ? "good" : s === "dropped" ? "idle" : s === "building" ? "warn" : "idle") as "good" | "warn" | "idle";

/** The plan as an outline of its items; the selected one opens beside it with its stage, jobs and notes. */
export function PlanView({
  planId,
  plan,
  st,
  itemId,
  reload,
}: {
  planId: string;
  plan: Plan;
  st: StageData;
  itemId: string | null;
  reload: () => void;
}) {
  const rows = useMemo(() => outline(plan), [plan]);
  const live = useMemo(() => liveItems(plan), [plan]);
  const counts = countBy(plan);
  const item = itemId ? plan.items.find((i) => i.id === itemId) : undefined;
  return (
    <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
      <section aria-label="Items" className={`${card} p-3`}>
        <p className="text-muted-foreground mb-2 text-[12px]">
          {Object.entries(counts)
            .filter(([, n]) => n > 0)
            .map(([k, n]) => `${n} ${k}`)
            .join(", ") || "No items yet."}
        </p>
        <ul className="flex flex-col gap-0.5">
          {rows.map(({ item: i, depth }) => (
            <li key={i.id} style={{ paddingLeft: depth * 16 }}>
              <a
                href={plannerHref({ planId, itemId: i.id })}
                aria-current={i.id === itemId ? "true" : undefined}
                className={cn(
                  "hover:bg-accent flex flex-wrap items-center gap-2 rounded-md px-2 py-1 text-[13px]",
                  i.id === itemId && "bg-accent",
                )}
              >
                <span className="text-muted-foreground text-[11px]">{i.kind}</span>
                <span className={cn(i.status === "dropped" && "line-through opacity-60")}>{i.title}</span>
                <Chip tone={tone(i.status)}>{st.stages.get(i.id)?.label ?? i.status}</Chip>
                {needsYou(i) ? <Chip tone="warn">needs you</Chip> : null}
                {live.has(i.id) ? (
                  <Chip tone="good" title={live.get(i.id)?.text}>
                    live
                  </Chip>
                ) : null}
              </a>
            </li>
          ))}
        </ul>
      </section>
      <section aria-label="Item" className={`${card} p-3`}>
        {item ? (
          <ItemDetail plan={plan} item={item} st={st} reload={reload} />
        ) : (
          <p className="text-muted-foreground text-[13px]">Pick an item.</p>
        )}
      </section>
    </div>
  );
}

function ItemDetail({ plan, item, st, reload }: { plan: Plan; item: Item; st: StageData; reload: () => void }) {
  const stage = st.stages.get(item.id);
  const jobs = st.jobsOf(item.id).length > 0 ? st.jobsOf(item.id) : itemJobs(item, []);
  const notes = plan.activity.filter((a) => a.item === item.id).slice(-8);
  return (
    <div className="flex flex-col gap-2">
      <h2 className="text-[15px] font-semibold">{item.title}</h2>
      <p className="text-muted-foreground text-[12px]">
        {item.kind} · {item.status}
        {stage ? ` · ${stageLine(stage)}` : ""}
        {item.by ? ` · ${item.by} ${ageOf(item.at)}` : ""}
      </p>
      {item.needs ? <p className="text-[13px] text-amber-700 dark:text-amber-400">Needs you: {item.needs}</p> : null}
      {item.body ? <p className="text-[13px] whitespace-pre-wrap">{item.body}</p> : null}
      {(stage?.prs ?? []).length > 0 && (
        <p className="flex flex-wrap gap-2 text-[12px]">
          {stage?.prs.map((p) => (
            <a
              key={`${p.repo}#${p.number}`}
              className="underline"
              href={prUrl(`${p.repo}#${p.number}`) || undefined}
              target="_blank"
              rel="noreferrer"
            >
              {p.repo}#{p.number}
            </a>
          ))}
        </p>
      )}
      {jobs.map((j) => (
        <JobPanel key={j} jobId={j} onMoved={reload} />
      ))}
      {notes.length > 0 && (
        <ul className="text-muted-foreground text-[12px]">
          {notes.map((n) => (
            <li key={`${n.at}${n.text}`}>
              {n.agent}: {n.text}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** Progress: where the plan is, counted by stage, with the steps that wait on you, a PR or a failure. */
export function ProgressView({ plan, st }: { plan: Plan; st: StageData }) {
  const p = useMemo(() => progressOf(plan, st.prs, Date.now(), st.stages), [plan, st]);
  return (
    <section aria-label="Progress" className="flex flex-col gap-3">
      <div className={`${card} p-3`}>
        <p className="text-[14px] font-medium">
          {p.percent === null ? "No steps yet" : `${p.percent}% of ${p.total} steps done`}
        </p>
        <div className="bg-muted mt-2 h-2 overflow-hidden rounded-full" aria-hidden>
          <div className="h-full bg-green-500" style={{ width: `${p.percent ?? 0}%` }} />
        </div>
        <p className="text-muted-foreground mt-2 text-[12px]">
          {STAGES.map((s) => `${p.byStage[s]} ${STAGE_LABEL[s].toLowerCase()}`).join(" · ")}
          {p.dropped ? ` · ${p.dropped} dropped` : ""}
          {p.elapsedMs !== null ? ` · ${duration(p.elapsedMs)} so far` : ""}
        </p>
        {p.current ? (
          <p className="mt-1 text-[13px]">
            Now: {p.current.item.title} ({STAGE_LABEL[p.current.stage]}
            {p.current.why ? `, ${p.current.why}` : ""})
          </p>
        ) : null}
        {st.reason ? <p className="text-muted-foreground mt-1 text-[12px]">Stages not read: {st.reason}</p> : null}
      </div>
      {p.waiting.length > 0 && (
        <ul className={`${card} flex flex-col gap-1 p-3 text-[13px]`} aria-label="Waiting">
          {p.waiting.map((s) => (
            <li key={s.item.id}>
              <span className="font-medium">{s.item.title}</span> {STAGE_LABEL[s.stage].toLowerCase()}
              {s.why ? `: ${s.why}` : ""}
              {decidablePrs(s.prs).map((r) => (
                <a
                  key={`${r.repo}#${r.number}`}
                  className="ml-2 underline"
                  href={prUrl(`${r.repo}#${r.number}`) || undefined}
                  target="_blank"
                  rel="noreferrer"
                >
                  review {r.repo}#{r.number}
                </a>
              ))}
            </li>
          ))}
        </ul>
      )}
      <ol className={`${card} p-3 text-[13px]`} aria-label="Steps">
        {p.steps.map((s) => (
          <li key={s.item.id} style={{ paddingLeft: s.depth * 16 }} className="flex gap-2 py-0.5">
            <Chip
              tone={
                s.stage === "done" ? "good" : s.stage === "failed" ? "bad" : s.stage === "not-started" ? "idle" : "warn"
              }
            >
              {STAGE_LABEL[s.stage]}
            </Chip>
            <span>{s.item.title}</span>
          </li>
        ))}
      </ol>
    </section>
  );
}

/** The plan canvas lives on its own page (the canvas port); this mode links to it and draws the 3D view's model from above until the scene canvas (#523) can take it. */
export function CanvasLink({ planId, plan, st }: { planId: string; plan: Plan; st: StageData }) {
  const scene = planScene(plan, st.stages);
  const e = scene.extent + 1;
  const at = (id: string) => scene.nodes.find((n) => n.id === id);
  const fill = { idle: "var(--muted-foreground)", good: "#16a34a", warn: "#d97706", bad: "#dc2626" } as const;
  return (
    <section aria-label="Plan canvas" className={`${card} p-4 text-[13px]`}>
      <p>The plan canvas shows this plan's items as cards you can move.</p>
      <a className="mt-2 inline-block underline" href={`/canvas/${encodeURIComponent(planId)}`}>
        Open the canvas
      </a>
      <svg
        role="img"
        aria-label="The plan from above, one dot per item"
        viewBox={`${-e} ${-e} ${2 * e} ${2 * e}`}
        className="mt-3 h-64 w-full"
      >
        {scene.edges.map((x) => {
          const a = at(x.from);
          const b = at(x.to);
          return a && b ? (
            <line
              key={`${x.from}-${x.to}`}
              x1={a.x}
              y1={a.z}
              x2={b.x}
              y2={b.z}
              stroke="currentColor"
              strokeOpacity={0.3}
            />
          ) : null;
        })}
        {scene.nodes.map((n) => (
          <circle key={n.id} cx={n.x} cy={n.z} r={0.45 * n.height} fill={fill[n.tone]}>
            <title>{`${n.title}: ${n.label}`}</title>
          </circle>
        ))}
      </svg>
      <p className="text-muted-foreground mt-2 text-[12px]">
        The 3D view is waiting for the scene page; this is the same plan from above.
      </p>
    </section>
  );
}
