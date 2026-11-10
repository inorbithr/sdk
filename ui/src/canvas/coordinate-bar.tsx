// The agent strip under the board: whether the live picture is connected, the asks this canvas
// filed (with their state), a quick suggestion on the selection and "Start work". Live comes from
// the coordinator's picture stream; writes go through the local server. The board draft itself is
// kept in this browser only, so the brief carries the cards' labels, not a saved canvas.
import { useRef, useState } from "react";
import { apiSend, useLive } from "../lib/live-shim.ts";
import { applyFrame, emptyPicture } from "../lib/picture.ts";
import type { Canvas } from "./schema.ts";
import {
  CHIPS, WORK_KINDS, asksFor, buildBrief, buildSelRef, isRepeat, suggestKind, suggestionAsk, suggestionText, workMarker,
  type ChipId, type WorkKind,
} from "./suggest-core.ts";

const LABEL = { connecting: "Connecting", live: "Live", reconnecting: "Reconnecting", off: "Off" } as const;

export function CoordinateBar({ canvas, sel }: { canvas: Canvas; sel: string[] }) {
  const { value: pic, state, reason } = useLive<unknown, ReturnType<typeof emptyPicture>>("/v1/coordinator/picture/events", {
    reduce: applyFrame,
    initial: emptyPicture(),
  });
  const ref = buildSelRef(canvas, sel);
  const asks = asksFor(pic.asks.values(), canvas.canvas_id);
  const [typed, setTyped] = useState("");
  const [note, setNote] = useState<string | null>(null);
  const [confirm, setConfirm] = useState(false);
  const [kind, setKind] = useState<WorkKind | null>(null);
  const recent = useRef(new Map<string, number>());

  const file = async (chip: ChipId | null) => {
    if (!ref) return;
    const text = suggestionText(chip, typed);
    const body = suggestionAsk(ref, text);
    if (!body) return;
    if (isRepeat(recent.current, ref, text, Date.now())) return;
    const r = await apiSend("POST", "/v1/coordinator/asks", { ...body, priority: 0, links: [] });
    setNote(r.ok ? "Filed with the coordinator." : `Not filed: ${r.reason}`);
    if (r.ok) setTyped("");
  };
  const start = async () => {
    const k = kind ?? suggestKind(ref);
    const marker = workMarker();
    const body = buildBrief({ canvas, ref, marker, kind: k, note: typed });
    const r = await apiSend("POST", "/v1/coordinator/asks", {
      title: `Start ${k} work: ${canvas.title}`.slice(0, 120),
      body,
      source: `canvas:${canvas.canvas_id}`,
      external_id: marker,
      priority: 0,
      links: [],
    });
    setConfirm(false);
    setNote(r.ok ? "Work queued. It ends in a pull request for review; nothing is merged or rolled." : `Not started: ${r.reason}`);
  };

  return (
    <div className="bg-card flex flex-col gap-2 border-t px-3 py-2 text-xs" aria-label="Coordinator">
      <div className="flex flex-wrap items-center gap-2">
        <span role="status" className="font-medium" title={reason ?? undefined}>
          Coordinator: {LABEL[state]}
          {state === "off" || state === "reconnecting" ? " (writes need the local server's live support)" : ""}
        </span>
        <span className="text-muted-foreground">{asks.length} filed from this canvas</span>
        <span className="text-muted-foreground ml-auto">Drafts stay in this browser; no canvas API yet.</span>
      </div>
      {asks.length ? (
        <ul className="flex flex-col gap-0.5">
          {asks.map((a) => (
            <li key={a.id} className="flex gap-2">
              <span className="bg-muted rounded px-1.5" data-state={a.state}>{a.label}</span>
              <span className="min-w-0 truncate">{a.text}</span>
            </li>
          ))}
        </ul>
      ) : null}
      {ref ? (
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-muted-foreground">On {ref.elements.length} selected:</span>
          {CHIPS.map((c) => (
            <button key={c.id} type="button" className="rounded-md border px-2 py-0.5" onClick={() => (c.prefill ? setTyped(c.prefill) : void file(c.id))}>
              {c.label}
            </button>
          ))}
          <input
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") void file(null); }}
            placeholder="Or say what to change, then Enter"
            aria-label="Suggestion"
            className="bg-background border-input min-w-40 flex-1 rounded-md border px-2 py-0.5"
          />
        </div>
      ) : null}
      <div className="flex flex-wrap items-center gap-2">
        {confirm ? (
          <>
            <select value={kind ?? suggestKind(ref)} onChange={(e) => setKind(e.target.value as WorkKind)} aria-label="Kind of work" className="bg-background rounded-md border px-1 py-0.5">
              {WORK_KINDS.map((k) => <option key={k.id} value={k.id}>{k.label}</option>)}
            </select>
            <span>Queue work for {ref ? "the selection" : "the whole canvas"}? It ends in a pull request; nothing is merged or rolled.</span>
            <button type="button" className="bg-primary text-primary-foreground rounded-md px-2 py-0.5" onClick={() => void start()}>Approve and start</button>
            <button type="button" className="rounded-md border px-2 py-0.5" onClick={() => setConfirm(false)}>Cancel</button>
          </>
        ) : (
          <button type="button" className="rounded-md border px-2 py-0.5" onClick={() => setConfirm(true)}>Start work…</button>
        )}
        {note ? <span role="status">{note}</span> : null}
      </div>
    </div>
  );
}
