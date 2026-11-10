import { useState } from "react";
import type { ApiPlanResponse } from "../lib/plan-api.ts";
import { ITEM_KINDS, type Plan, STATUSES } from "../lib/planner.ts";
import { claimBody, editProblem, itemPath, newItemId, noteBody, upsertBody } from "../lib/planner-write.ts";
import { send } from "./planner-data.ts";
import { button, card, field } from "./ui.tsx";

export const HOLDER = "iohr-ui";

/** Write: change an item, add one, say something on it, claim it. Each write names the version it read, so a newer save is refused, not overwritten. */
export function WriteView({
  planId,
  plan,
  raw,
  itemId,
  reload,
}: {
  planId: string;
  plan: Plan;
  raw: ApiPlanResponse | null;
  itemId: string | null;
  reload: () => void;
}) {
  const had = raw?.items?.find((i) => i.item_id === itemId);
  const item = plan.items.find((i) => i.id === itemId);
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [status, setStatus] = useState("");
  const [note, setNote] = useState("");
  const [fresh, setFresh] = useState({ title: "", kind: "step" });
  const [msg, setMsg] = useState("");
  const [busy, setBusy] = useState(false);
  const done = (r: { ok: boolean; reason?: string }, ok: string) => {
    setBusy(false);
    setMsg(r.ok ? ok : (r.reason ?? "Not saved."));
    reload();
  };
  const save = async () => {
    if (!itemId || !had) return;
    const edit = { ...(title ? { title } : {}), ...(body ? { body } : {}), ...(status ? { status } : {}) };
    const problem = editProblem(edit);
    if (problem) return setMsg(problem);
    setBusy(true);
    done(await send("PUT", itemPath(planId, itemId), upsertBody(had, edit, HOLDER)), "Saved.");
  };
  const add = async () => {
    const problem = editProblem(fresh);
    if (problem) return setMsg(problem);
    setBusy(true);
    const id = newItemId(fresh.title, new Set(plan.items.map((i) => i.id)));
    const r = await send("PUT", itemPath(planId, id), upsertBody(undefined, fresh, HOLDER));
    if (r.ok) setFresh({ title: "", kind: "step" });
    done(r, "Added.");
  };
  const say = async () => {
    if (!itemId || note.trim() === "") return;
    setBusy(true);
    const r = await send("POST", itemPath(planId, itemId, "/notes"), noteBody(note));
    if (r.ok) setNote("");
    done(r, "Noted.");
  };
  const claim = async (tail: "/claim" | "/release") => {
    if (!itemId) return;
    setBusy(true);
    done(
      await send("POST", itemPath(planId, itemId, tail), claimBody(HOLDER)),
      tail === "/claim" ? "Claimed." : "Released.",
    );
  };
  return (
    <section aria-label="Write" className="grid gap-4 lg:grid-cols-2">
      <div className={`${card} flex flex-col gap-2 p-3`}>
        <h2 className="text-[14px] font-medium">{item ? item.title : "Pick an item to change"}</h2>
        {item ? (
          <>
            <input
              aria-label="Title"
              placeholder={item.title}
              className={field}
              value={title}
              onChange={(e) => setTitle(e.target.value)}
            />
            <textarea
              aria-label="Text"
              placeholder={item.body || "Text"}
              rows={5}
              className={field}
              value={body}
              onChange={(e) => setBody(e.target.value)}
            />
            <select
              aria-label="Status"
              className={`${field} w-auto`}
              value={status}
              onChange={(e) => setStatus(e.target.value)}
            >
              <option value="">Keep {item.status}</option>
              {STATUSES.map((s) => (
                <option key={s}>{s}</option>
              ))}
            </select>
            <div className="flex gap-2">
              <button type="button" className={button} disabled={busy || !had} onClick={() => void save()}>
                Save
              </button>
              <button type="button" className={button} disabled={busy} onClick={() => void claim("/claim")}>
                Claim
              </button>
              <button type="button" className={button} disabled={busy} onClick={() => void claim("/release")}>
                Release
              </button>
            </div>
            <input
              aria-label="Note"
              placeholder="Say something on this item"
              className={field}
              value={note}
              onChange={(e) => setNote(e.target.value)}
            />
            <button type="button" className={button} disabled={busy || note.trim() === ""} onClick={() => void say()}>
              Add note
            </button>
          </>
        ) : (
          <p className="text-muted-foreground text-[13px]">Open an item from the plan first.</p>
        )}
      </div>
      <div className={`${card} flex flex-col gap-2 p-3`}>
        <h2 className="text-[14px] font-medium">Add an item</h2>
        <input
          aria-label="New title"
          placeholder="Title"
          className={field}
          value={fresh.title}
          onChange={(e) => setFresh({ ...fresh, title: e.target.value })}
        />
        <select
          aria-label="New kind"
          className={`${field} w-auto`}
          value={fresh.kind}
          onChange={(e) => setFresh({ ...fresh, kind: e.target.value })}
        >
          {ITEM_KINDS.map((k) => (
            <option key={k}>{k}</option>
          ))}
        </select>
        <button type="button" className={button} disabled={busy} onClick={() => void add()}>
          Add
        </button>
      </div>
      {msg ? (
        <p role="status" className="text-[13px] lg:col-span-2">
          {msg}
        </p>
      ) : null}
    </section>
  );
}
