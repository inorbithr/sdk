import { useEffect, useState } from "react";
import { apiGet } from "../lib/api.ts";
import { actionPath, controlsFor, holdCause, type JobAction } from "../lib/job-controls.ts";
import { type JobLike, parseJob, parseTransitions, type TransitionLike } from "../lib/planner-shared.ts";
import { reasonBody } from "../lib/planner-write.ts";
import { send } from "./planner-data.ts";
import { button, Chip, field } from "./ui.tsx";

type Loaded = { job: JobLike | null; moves: TransitionLike[]; reason: string };

/** One coordinator job: its stage, why it is held, its moves, and the controls the mode allows (each needs a reason). */
export function JobPanel({ jobId, onMoved }: { jobId: string; onMoved: () => void }) {
  const [d, setD] = useState<Loaded | null>(null);
  const [tick, setTick] = useState(0);
  const [why, setWhy] = useState("");
  const [msg, setMsg] = useState("");
  const [busy, setBusy] = useState(false);
  // biome-ignore lint/correctness/useExhaustiveDependencies: `tick` re-reads after a move
  useEffect(() => {
    let live = true;
    const id = encodeURIComponent(jobId);
    void Promise.all([
      apiGet<unknown>(`/v1/coordinator/jobs/${id}`),
      apiGet<unknown>(`/v1/coordinator/jobs/${id}/transitions`),
    ]).then(([j, t]) => {
      if (!live) return;
      const raw = j.state === "ok" ? ((j.data as { job?: unknown }).job ?? j.data) : null;
      const tr = t.state === "ok" ? (t.data as { transitions?: unknown }).transitions : [];
      setD({
        job: parseJob(raw),
        moves: parseTransitions(tr),
        reason: j.state === "ok" ? "" : j.state === "signed-out" ? "Signed out." : j.reason,
      });
    });
    return () => {
      live = false;
    };
  }, [jobId, tick]);
  if (d === null) return <p className="text-muted-foreground text-[12px]">Reading {jobId}…</p>;
  if (d.job === null)
    return (
      <p className="text-[12px]">
        {jobId}: {d.reason || "not found"}
      </p>
    );
  const job = d.job;
  const run = async (action: JobAction) => {
    const path = actionPath(action, job.jobId);
    if (path === null) return;
    if (why.trim() === "") {
      setMsg("Say why first: the coordinator refuses a move without a reason.");
      return;
    }
    setBusy(true);
    const r = await send("POST", path, reasonBody(why));
    setBusy(false);
    setMsg(r.ok ? `${action} sent.` : r.reason);
    if (r.ok) {
      setWhy("");
      setTick((t) => t + 1);
      onMoved();
    }
  };
  const cause = holdCause(job);
  // Refile queues a new ask with a request shape the web has not verified, so only the machine moves are offered.
  const moves = controlsFor(job).filter((c) => actionPath(c.action, job.jobId) !== null);
  return (
    <div className="flex flex-col gap-1.5 text-[12px]" data-job={job.jobId}>
      <p className="flex flex-wrap items-center gap-2">
        <code>{job.jobId}</code>
        <Chip>{job.stage || "no stage"}</Chip>
        <Chip>{job.mode || job.state || "no mode"}</Chip>
        {job.pullRequest ? (
          <span>
            {job.pullRequest}
            {job.prState ? ` (${job.prState})` : ""}
          </span>
        ) : null}
      </p>
      {cause ? <p className="text-amber-700 dark:text-amber-400">{cause}</p> : null}
      {moves.length > 0 && (
        <div className="flex flex-wrap items-center gap-2">
          <input
            aria-label="Reason"
            placeholder="Why"
            className={`${field} w-56`}
            value={why}
            onChange={(e) => setWhy(e.target.value)}
          />
          {moves.map((c) => (
            <button
              key={c.action}
              type="button"
              title={c.hint}
              disabled={busy}
              className={button}
              onClick={() => void run(c.action)}
            >
              {c.label}
            </button>
          ))}
        </div>
      )}
      {msg ? <p role="status">{msg}</p> : null}
      {d.moves.length > 0 && (
        <ol className="text-muted-foreground">
          {d.moves.slice(-5).map((m) => (
            <li key={m.seq}>
              {m.fromStage || "start"} to {m.toStage} ({m.kind}, {m.actor}){m.detail ? `: ${m.detail}` : ""}
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}
