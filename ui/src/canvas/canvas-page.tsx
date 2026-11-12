// The Canvas page (PRD 0065, RFC 0217): the board of the Mac app on the web, drawn natively (DOM and SVG over the
// same `iohr.canvas` v3 file) because the Mac app's patched Excalidraw is not ported. Core has no canvas API yet, so
// a canvas is kept in this browser's storage, per viewer; the page says so. Slice 1: list, open, draw, edit, undo.
import {
  ArrowUpRight,
  Circle,
  Copy,
  Frame,
  Hand,
  MousePointer2,
  Plus,
  Redo2,
  Square,
  StickyNote,
  Trash2,
  Undo2,
} from "lucide-react";
import {
  type ReactNode,
  type PointerEvent as RPointer,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { cn } from "../frame/cn.ts";
import { showContextMenu } from "../frame/context-menu.tsx";
import { FIRST_PANEL } from "../frame/dock-frame.ts";
import { useShortcuts } from "../lib/use-shortcuts.ts";
import {
  addItems,
  boundsOf,
  connect,
  duplicate,
  editTarget,
  endpoints,
  HANDLES,
  type Handle,
  hitTest,
  itemDomId,
  itemLabel,
  itemsInRect,
  makeFrame,
  makeNote,
  makeShape,
  moveEnd,
  moveItems,
  normRect,
  reorder,
  resizeBox,
  resizeItem,
  setText,
  setTitle,
  setTone,
  snap,
} from "./board-core.ts";
import {
  browserKV,
  copyCanvas,
  createCanvas,
  deleteCanvas,
  listCanvases,
  readView,
  rememberCanvas,
  renameCanvas,
  writeView,
} from "./board-store-core.ts";
import { boardMenu, itemMenu } from "./canvas-menus.ts";
import { canvasPath, canvasRoute } from "./canvas-route.ts";
import { Card, isCard } from "./cards.tsx";
import { CoordinateBar } from "./coordinate-bar.tsx";
import { Inspector } from "./inspector.tsx";
import { type CardKind, pick as pickCard } from "./picker-core.ts";
import { type Box, type Item, paintOrder, removeItems, type ShapeItem, type Tone } from "./schema.ts";
import { useBoard } from "./use-board.ts";
import { detailAt, fit, freeSpot, linkPath, toBoard, type View, zoomAt } from "./viewport.ts";

const KEPT_HERE = "Kept in this browser only: core has no canvas API yet, so nothing here is saved to your account.";

type Tool = "select" | "note" | "frame" | "rectangle" | "ellipse" | "arrow";

const TONE: Record<Tone, string> = {
  plain: "bg-card border-border",
  amber: "bg-amber-100 border-amber-300 text-amber-950",
  green: "bg-green-100 border-green-300 text-green-950",
  blue: "bg-blue-100 border-blue-300 text-blue-950",
  violet: "bg-violet-100 border-violet-300 text-violet-950",
  red: "bg-red-100 border-red-300 text-red-950",
};

export default function CanvasPage() {
  const route = canvasRoute(window.location.pathname);
  if (!route) return null;
  return route.id === null ? <CanvasList /> : <Editor key={route.id} id={route.id} />;
}

const go = (id: string | null) => window.location.assign(canvasPath(id));

// ---------------------------------------------------------------- the list

function CanvasList() {
  const kv = useMemo(() => browserKV(), []);
  const [list, setList] = useState(() => listCanvases(kv));
  const [title, setTitle] = useState("");
  return (
    <section className="flex max-w-2xl flex-col gap-4" aria-label="Canvases">
      <p className="text-muted-foreground text-[13px]" role="note">
        {KEPT_HERE}
      </p>
      <form
        className="flex gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          go(createCanvas(kv, title).canvas_id);
        }}
      >
        <input
          className="bg-background border-input min-w-0 flex-1 rounded-md border px-2 py-1 text-sm"
          placeholder="Name a new canvas"
          aria-label="Canvas name"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
        />
        <button type="submit" className="bg-primary text-primary-foreground rounded-md px-3 py-1 text-sm">
          New canvas
        </button>
      </form>
      {list.length === 0 ? <p className="text-muted-foreground text-sm">No canvas yet.</p> : null}
      <ul className="flex flex-col divide-y rounded-md border">
        {list.map((c) => (
          <li key={c.canvas_id} className="flex items-center gap-2 px-3 py-2">
            <button type="button" className="min-w-0 flex-1 text-left" onClick={() => go(c.canvas_id)}>
              <span className="block truncate text-sm font-medium">{c.title}</span>
              <span className="text-muted-foreground block text-[12px]">
                {c.items} items, saved {c.updated_at.slice(0, 16).replace("T", " ")}
              </span>
            </button>
            <IconButton
              label="Rename"
              onClick={() => {
                const t = window.prompt("Rename the canvas", c.title);
                if (t) {
                  renameCanvas(kv, c.canvas_id, t);
                  setList(listCanvases(kv));
                }
              }}
            >
              <span className="text-[12px]">Rename</span>
            </IconButton>
            <IconButton
              label="Duplicate"
              onClick={() => {
                copyCanvas(kv, c.canvas_id);
                setList(listCanvases(kv));
              }}
            >
              <Copy />
            </IconButton>
            <IconButton
              label="Delete"
              onClick={() => {
                if (window.confirm(`Delete "${c.title}" from this browser?`)) {
                  deleteCanvas(kv, c.canvas_id);
                  setList(listCanvases(kv));
                }
              }}
            >
              <Trash2 />
            </IconButton>
          </li>
        ))}
      </ul>
    </section>
  );
}

function IconButton({
  label,
  onClick,
  children,
  active,
  disabled,
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
  active?: boolean;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-pressed={active}
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "hover:bg-accent text-muted-foreground hover:text-foreground flex items-center rounded-md p-1.5 disabled:opacity-40 [&>svg]:size-4",
        active && "bg-accent text-foreground",
      )}
    >
      {children}
    </button>
  );
}

// ---------------------------------------------------------------- the editor

type Drag =
  | { kind: "pan"; sx: number; sy: number; from: View }
  | { kind: "move"; last: { x: number; y: number }; moved: boolean }
  | { kind: "resize"; id: string; handle: Handle; start: Box; from: { x: number; y: number } }
  | { kind: "end"; id: string; which: "start" | "end" }
  | { kind: "marquee"; from: { x: number; y: number }; to: { x: number; y: number }; add: boolean }
  | { kind: "draw"; tool: Tool; from: { x: number; y: number }; to: { x: number; y: number }; over: string | null };

function Editor({ id }: { id: string }) {
  const { board, snap: st } = useBoard(id);
  const kv = useMemo(() => browserKV(), []);
  const host = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 800, h: 600 });
  const [view, setView] = useState<View>(() => readView(kv, id) ?? { x: 40, y: 40, zoom: 1 });
  const [tool, setTool] = useState<Tool>("select");
  const [sel, setSel] = useState<string[]>([]);
  const [drag, setDrag] = useState<Drag | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [space, setSpace] = useState(false);
  const [panel, setPanel] = useState<{ open: boolean; width: number }>({ open: false, width: FIRST_PANEL });
  const [adding, setAdding] = useState(false);
  const canvas = st.canvas;
  const ready = canvas !== null;

  useEffect(() => {
    rememberCanvas(kv, id);
  }, [kv, id]);
  useEffect(() => {
    const el = host.current;
    if (!ready || !el) return;
    const ro = new ResizeObserver(() => setSize({ w: el.clientWidth, h: el.clientHeight }));
    ro.observe(el);
    setSize({ w: el.clientWidth, h: el.clientHeight });
    return () => ro.disconnect();
  }, [ready]);
  useEffect(() => {
    const t = setTimeout(() => writeView(kv, id, view), 300);
    return () => clearTimeout(t);
  }, [kv, id, view]);

  const edit = useCallback(
    (f: Parameters<NonNullable<typeof board>["change"]>[0], undoable = true) => board?.change(f, { undoable }),
    [board],
  );
  const at = (e: { clientX: number; clientY: number }) => {
    const r = host.current?.getBoundingClientRect();
    return toBoard(view, e.clientX - (r?.left ?? 0), e.clientY - (r?.top ?? 0));
  };

  const place = (make: (p: { x: number; y: number }) => Item) => {
    if (!canvas) return;
    const item = make(freeSpot(view, size.w, size.h, { w: 220, h: 150 }, canvas.items));
    edit((c) => addItems(c, [item]));
    setSel([item.item_id]);
  };
  const remove = () => {
    if (sel.length) edit((c) => removeItems(c, sel));
    setSel([]);
  };
  const fitTo = (b: Box | null) => setView(fit(b, size.w, size.h));
  const pick = (t: Tool) => {
    setTool(t);
    if (t === "note") place((p) => makeNote(p));
    if (t === "frame") place((p) => makeFrame(p));
    if (t === "note" || t === "frame") setTool("select");
  };

  const duplicateSel = () => {
    if (!canvas || !sel.length) return;
    const d = duplicate(canvas, sel);
    edit(() => d.canvas);
    setSel(d.ids);
  };
  const menuAt = (e: React.MouseEvent) => {
    e.preventDefault();
    if (!canvas) return;
    const p = at(e);
    const hit = hitTest(canvas.items, p, 8 / view.zoom);
    let ids = sel;
    if (hit && !sel.includes(hit.item_id)) {
      ids = [hit.item_id];
      setSel(ids);
    }
    const rows =
      hit || ids.length
        ? itemMenu(
            canvas.items.filter((i) => ids.includes(i.item_id)),
            {
              edit: () => setEditing(ids[0] ?? null),
              inspect: () => setPanel((x) => ({ ...x, open: true })),
              duplicate: duplicateSel,
              front: () => edit((c) => reorder(c, ids, "front")),
              back: () => edit((c) => reorder(c, ids, "back")),
              tone: (t) => edit((c) => setTone(c, ids, t)),
              fit: () => fitTo(boundsOf(canvas, ids)),
              remove: remove,
            },
          )
        : boardMenu({
            note: () => edit((c) => addItems(c, [makeNote(at(e))])),
            frame: () => edit((c) => addItems(c, [makeFrame(at(e))])),
            add: () => setAdding(true),
            fitAll: () => fitTo(boundsOf(canvas)),
            fitHome: () => setView({ x: 40, y: 40, zoom: 1 }),
            undo: st.past ? () => board?.undo() : null,
            redo: st.future ? () => board?.redo() : null,
          });
    showContextMenu(e, rows, { label: hit ? "Item" : "Canvas" });
  };

  useShortcuts("canvas", {
    "canvas-undo": () => board?.undo(),
    "canvas-redo": () => board?.redo(),
    "canvas-fit-home": () => setView({ x: 40, y: 40, zoom: 1 }),
    "canvas-fit-selection": () => fitTo(canvas ? boundsOf(canvas, sel) : null),
    "canvas-fit-all": () => fitTo(canvas ? boundsOf(canvas) : null),
    "canvas-delete": remove,
    "canvas-edit": () => {
      const id = canvas ? editTarget(canvas, sel) : null;
      if (id) setEditing(id);
    },
    "canvas-note": () => pick("note"),
    "canvas-arrow": () => setTool("arrow"),
    "canvas-cancel": () => {
      setTool("select");
      setSel([]);
      setEditing(null);
    },
  });
  useEffect(() => {
    const dn = (e: KeyboardEvent) => {
      if (e.code === "Space" && !(e.target instanceof HTMLTextAreaElement) && !(e.target instanceof HTMLInputElement)) {
        setSpace(true);
        e.preventDefault();
      }
    };
    const up = (e: KeyboardEvent) => e.code === "Space" && setSpace(false);
    window.addEventListener("keydown", dn);
    window.addEventListener("keyup", up);
    return () => {
      window.removeEventListener("keydown", dn);
      window.removeEventListener("keyup", up);
    };
  }, []);

  if (st.error && !canvas) {
    return (
      <section className="notice" role="alert">
        <p>{st.error}</p>
        <button type="button" className="underline" onClick={() => go(null)}>
          Back to the canvases
        </button>
      </section>
    );
  }
  if (!canvas) return <p className="text-muted-foreground text-sm">Opening the canvas…</p>;

  const byId = new Map(canvas.items.map((i) => [i.item_id, i]));
  const single = sel.length === 1 ? byId.get(sel[0]) : undefined;

  const down = (e: RPointer<HTMLDivElement>) => {
    if (editing) return;
    host.current?.setPointerCapture(e.pointerId);
    const p = at(e);
    if (e.button === 1 || space || (tool === "select" && e.button !== 0)) {
      setDrag({ kind: "pan", sx: e.clientX, sy: e.clientY, from: view });
      return;
    }
    const hit = hitTest(canvas.items, p, 8 / view.zoom);
    if (tool === "select") {
      if (hit) {
        const next = e.shiftKey
          ? sel.includes(hit.item_id)
            ? sel.filter((s) => s !== hit.item_id)
            : [...sel, hit.item_id]
          : sel.includes(hit.item_id)
            ? sel
            : [hit.item_id];
        setSel(next);
        board?.checkpoint();
        setDrag({ kind: "move", last: p, moved: false });
      } else {
        if (!e.shiftKey) setSel([]);
        setDrag({ kind: "marquee", from: p, to: p, add: e.shiftKey });
      }
      return;
    }
    setDrag({ kind: "draw", tool, from: p, to: p, over: hit?.item_id ?? null });
  };

  const move = (e: RPointer<HTMLDivElement>) => {
    if (!drag) return;
    const p = at(e);
    if (drag.kind === "pan")
      setView({ ...drag.from, x: drag.from.x + e.clientX - drag.sx, y: drag.from.y + e.clientY - drag.sy });
    else if (drag.kind === "move") {
      const dx = p.x - drag.last.x,
        dy = p.y - drag.last.y;
      if (dx || dy) {
        edit((c) => moveItems(c, sel, dx, dy), false);
        setDrag({ ...drag, last: p, moved: true });
      }
    } else if (drag.kind === "resize") {
      const b = resizeBox(drag.start, drag.handle, p.x - drag.from.x, p.y - drag.from.y);
      edit((c) => resizeItem(c, drag.id, b), false);
    } else if (drag.kind === "end") edit((c) => moveEnd(c, drag.id, drag.which, p), false);
    else if (drag.kind === "marquee") setDrag({ ...drag, to: p });
    else if (drag.kind === "draw") setDrag({ ...drag, to: p });
  };

  const up = (e: RPointer<HTMLDivElement>) => {
    if (!drag) return;
    host.current?.releasePointerCapture(e.pointerId);
    const p = at(e);
    if (drag.kind === "move" && drag.moved) edit((c) => moveItems(c, [], 0, 0), false);
    if (drag.kind === "marquee") {
      const hits = itemsInRect(canvas.items, normRect(drag.from, p));
      setSel(drag.add ? [...new Set([...sel, ...hits])] : hits);
    }
    if (drag.kind === "draw") finishDraw(drag, p);
    setDrag(null);
  };

  const finishDraw = (d: Extract<Drag, { kind: "draw" }>, p: { x: number; y: number }) => {
    const r = normRect(d.from, p);
    const small = r.w < 6 && r.h < 6;
    if (d.tool === "arrow") {
      const over = hitTest(canvas.items, p, 8 / view.zoom)?.item_id ?? null;
      if (d.over && over && d.over !== over) {
        edit((c) => connect(c, d.over as string, over));
      } else if (!small) {
        const a = makeShape("arrow", { x: d.from.x, y: d.from.y, w: p.x - d.from.x, h: p.y - d.from.y });
        const box = normRect(d.from, p);
        const s: ShapeItem = {
          ...a,
          x: box.x,
          y: box.y,
          w: box.w,
          h: box.h,
          points: [
            [d.from.x - box.x, d.from.y - box.y],
            [p.x - box.x, p.y - box.y],
          ],
        };
        edit((c) => addItems(c, [s]));
        setSel([s.item_id]);
      }
    } else {
      const shape = makeShape(
        d.tool as "rectangle" | "ellipse",
        small
          ? { x: snap(d.from.x), y: snap(d.from.y), w: 160, h: 100 }
          : { x: snap(r.x), y: snap(r.y), w: snap(r.w), h: snap(r.h) },
      );
      edit((c) => addItems(c, [shape]));
      setSel([shape.item_id]);
    }
    setTool("select");
  };

  const wheel = (e: React.WheelEvent) => {
    const r = host.current?.getBoundingClientRect();
    const sx = e.clientX - (r?.left ?? 0),
      sy = e.clientY - (r?.top ?? 0);
    if (e.ctrlKey || e.metaKey) setView(zoomAt(view, sx, sy, Math.exp(-e.deltaY * 0.01)));
    else setView({ ...view, x: view.x - e.deltaX, y: view.y - e.deltaY });
  };

  const startResize = (e: RPointer, it: Item, handle: Handle) => {
    e.stopPropagation();
    host.current?.setPointerCapture(e.pointerId);
    board?.checkpoint();
    setDrag({ kind: "resize", id: it.item_id, handle, start: { x: it.x, y: it.y, w: it.w, h: it.h }, from: at(e) });
  };
  const startEnd = (e: RPointer, it: Item, which: "start" | "end") => {
    e.stopPropagation();
    host.current?.setPointerCapture(e.pointerId);
    board?.checkpoint();
    setDrag({ kind: "end", id: it.item_id, which });
  };

  const detail = detailAt(view.zoom);
  const cursor = drag?.kind === "pan" || space ? "grabbing" : tool === "select" ? "default" : "crosshair";

  return (
    <section className="flex h-full min-h-[480px] flex-col gap-2" aria-label={canvas.title}>
      <header className="flex flex-wrap items-center gap-1">
        <button
          type="button"
          className="text-muted-foreground hover:text-foreground text-[13px]"
          onClick={() => go(null)}
        >
          Canvases
        </button>
        <span className="text-muted-foreground">/</span>
        <span className="mr-2 truncate text-sm font-medium">{canvas.title}</span>
        <IconButton label="Add a card (document, pull request, decision)" onClick={() => setAdding(!adding)}>
          <Plus />
        </IconButton>
        <IconButton label="Select" active={tool === "select"} onClick={() => setTool("select")}>
          <MousePointer2 />
        </IconButton>
        <IconButton label="Sticky note (N)" onClick={() => pick("note")}>
          <StickyNote />
        </IconButton>
        <IconButton label="Frame" onClick={() => pick("frame")}>
          <Frame />
        </IconButton>
        <IconButton label="Rectangle" active={tool === "rectangle"} onClick={() => setTool("rectangle")}>
          <Square />
        </IconButton>
        <IconButton label="Ellipse" active={tool === "ellipse"} onClick={() => setTool("ellipse")}>
          <Circle />
        </IconButton>
        <IconButton label="Arrow (A)" active={tool === "arrow"} onClick={() => setTool("arrow")}>
          <ArrowUpRight />
        </IconButton>
        <span className="bg-border mx-1 h-4 w-px" />
        <IconButton label="Undo" disabled={!st.past} onClick={() => board?.undo()}>
          <Undo2 />
        </IconButton>
        <IconButton label="Redo" disabled={!st.future} onClick={() => board?.redo()}>
          <Redo2 />
        </IconButton>
        <IconButton label="Delete the selection" disabled={!sel.length} onClick={remove}>
          <Trash2 />
        </IconButton>
        <IconButton label="Fit everything" onClick={() => fitTo(boundsOf(canvas))}>
          <Hand />
        </IconButton>
        <span className="text-muted-foreground ml-auto text-[12px]" role="status">
          {Math.round(view.zoom * 100)}% · {st.error ?? (st.dirty ? "Saving…" : "Saved in this browser")}
        </span>
      </header>
      <p className="text-muted-foreground text-[12px]" role="note">
        {KEPT_HERE}
      </p>
      {adding ? (
        <AddCard
          onClose={() => setAdding(false)}
          onAdd={(make) => {
            place(make);
            setAdding(false);
          }}
        />
      ) : null}
      <div className="flex min-h-0 flex-1">
        <div
          ref={host}
          data-canvas-board
          role="listbox"
          aria-label="Canvas board"
          aria-multiselectable="true"
          aria-activedescendant={sel[0] ? itemDomId(sel[0]) : undefined}
          className="bg-muted/40 relative min-h-0 min-w-0 flex-1 touch-none overflow-hidden rounded-md border outline-none"
          style={{ cursor }}
          tabIndex={0}
          onPointerDown={down}
          onPointerMove={move}
          onPointerUp={up}
          onWheel={wheel}
          onContextMenu={menuAt}
        >
          <div
            className="absolute left-0 top-0"
            style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${view.zoom})`, transformOrigin: "0 0" }}
          >
            <svg
              className="pointer-events-none absolute left-0 top-0 overflow-visible"
              width={1}
              height={1}
              aria-hidden="true"
            >
              <defs>
                <marker id="cv-head" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto">
                  <path d="M0,0 L8,4 L0,8 z" fill="currentColor" />
                </marker>
              </defs>
              {canvas.links.map((l) => {
                const a = byId.get(l.from.item_id),
                  b = byId.get(l.to.item_id);
                return a && b ? (
                  <path
                    key={l.link_id}
                    d={linkPath(a, b).d}
                    fill="none"
                    stroke="currentColor"
                    strokeWidth={1.5}
                    className="text-muted-foreground"
                  />
                ) : null;
              })}
              {canvas.items.map((i) =>
                i.type === "shape" && (i.shape === "arrow" || i.shape === "line") ? (
                  <Line key={i.item_id} it={i} on={sel.includes(i.item_id)} />
                ) : null,
              )}
              {drag?.kind === "draw" && drag.tool === "arrow" ? (
                <line
                  x1={drag.from.x}
                  y1={drag.from.y}
                  x2={drag.to.x}
                  y2={drag.to.y}
                  stroke="currentColor"
                  strokeWidth={1.5}
                  markerEnd="url(#cv-head)"
                  strokeDasharray="4 3"
                />
              ) : null}
            </svg>
            {paintOrder(canvas.items).map((i) =>
              i.type === "shape" && (i.shape === "arrow" || i.shape === "line") ? null : (
                <ItemView
                  key={i.item_id}
                  it={i}
                  detail={detail}
                  selected={sel.includes(i.item_id)}
                  editing={editing === i.item_id}
                  onEdit={() =>
                    (i.type === "note" || i.type === "frame" || i.type === "shape") && setEditing(i.item_id)
                  }
                  onText={(t) => edit((c) => setText(c, i.item_id, t))}
                  onDone={() => setEditing(null)}
                />
              ),
            )}
            {single && single.type === "shape" && (single.shape === "arrow" || single.shape === "line")
              ? (["start", "end"] as const).map((w) => {
                  const ends = endpoints(single);
                  const pt = ends ? ends[w === "start" ? 0 : 1] : null;
                  return pt ? (
                    <span
                      key={w}
                      role="presentation"
                      className="border-primary bg-background absolute size-3 -translate-x-1/2 -translate-y-1/2 cursor-move rounded-full border-2"
                      style={{ left: pt.x, top: pt.y }}
                      onPointerDown={(e) => startEnd(e, single, w)}
                    />
                  ) : null;
                })
              : single &&
                  !(single.type === "shape" && (single.shape === "arrow" || single.shape === "line")) &&
                  !editing
                ? HANDLES.map((h) => (
                    <HandleDot
                      key={h}
                      it={single}
                      handle={h}
                      zoom={view.zoom}
                      onDown={(e) => startResize(e, single, h)}
                    />
                  ))
                : null}
            {drag?.kind === "marquee" ? <Rect r={normRect(drag.from, drag.to)} /> : null}
            {drag?.kind === "draw" && drag.tool !== "arrow" ? <Rect r={normRect(drag.from, drag.to)} /> : null}
          </div>
          {canvas.items.length === 0 ? (
            <p className="text-muted-foreground pointer-events-none absolute inset-0 flex items-center justify-center text-sm">
              An empty canvas. Press N for a sticky note, or pick a tool above.
            </p>
          ) : null}
        </div>
        {panel.open && single ? (
          <Inspector
            item={single}
            width={panel.width}
            onWidth={(w) => setPanel((x) => ({ ...x, width: w }))}
            onClose={() => setPanel((x) => ({ ...x, open: false }))}
            onTitle={(t) => edit((c) => setTitle(c, single.item_id, t))}
            onText={(t) => edit((c) => setText(c, single.item_id, t))}
            onTone={(t) => edit((c) => setTone(c, [single.item_id], t))}
          />
        ) : null}
        <CoordinateBar canvas={canvas} sel={sel} />
      </div>
    </section>
  );
}

function AddCard({
  onAdd,
  onClose,
}: {
  onAdd: (make: (at: { x: number; y: number }) => Item) => void;
  onClose: () => void;
}) {
  const [kind, setKind] = useState<CardKind>("decision");
  const [text, setText] = useState("");
  const [why, setWhy] = useState("");
  const hint = { decision: "RFC 0217", pr: "owner/repo#12", document: "space id, a space, document id" }[kind];
  return (
    <form
      className="flex flex-wrap items-center gap-2 text-sm"
      onSubmit={(e) => {
        e.preventDefault();
        const r = pickCard(kind, text);
        if (r.ok) onAdd(r.make);
        else setWhy(r.why);
      }}
    >
      <select
        aria-label="Kind of card"
        className="bg-background border-input rounded-md border px-2 py-1"
        value={kind}
        onChange={(e) => setKind(e.target.value as CardKind)}
      >
        <option value="decision">Decision</option>
        <option value="pr">Pull request</option>
        <option value="document">Document</option>
      </select>
      <input
        aria-label="Reference"
        className="bg-background border-input min-w-0 flex-1 rounded-md border px-2 py-1"
        placeholder={hint}
        value={text}
        onChange={(e) => setText(e.target.value)}
      />
      <button type="submit" className="bg-primary text-primary-foreground rounded-md px-3 py-1">
        Add
      </button>
      <button type="button" className="text-muted-foreground" onClick={onClose}>
        Cancel
      </button>
      {why ? (
        <span role="alert" className="text-destructive text-[12px]">
          {why}
        </span>
      ) : null}
    </form>
  );
}

const Rect = ({ r }: { r: Box }) => (
  <div
    className="border-primary bg-primary/10 pointer-events-none absolute border"
    style={{ left: r.x, top: r.y, width: r.w, height: r.h }}
  />
);

function Line({ it, on }: { it: ShapeItem; on: boolean }) {
  const e = endpoints(it);
  if (!e) return null;
  return (
    <line
      x1={e[0].x}
      y1={e[0].y}
      x2={e[1].x}
      y2={e[1].y}
      stroke={it.style?.stroke ?? "currentColor"}
      strokeWidth={on ? 3 : 1.5}
      markerEnd={it.shape === "arrow" ? "url(#cv-head)" : undefined}
      className={on ? "text-primary" : "text-foreground"}
    />
  );
}

function HandleDot({
  it,
  handle,
  zoom,
  onDown,
}: {
  it: Item;
  handle: Handle;
  zoom: number;
  onDown: (e: RPointer) => void;
}) {
  const x = handle.includes("w") ? it.x : handle.includes("e") ? it.x + it.w : it.x + it.w / 2;
  const y = handle.includes("n") ? it.y : handle.includes("s") ? it.y + it.h : it.y + it.h / 2;
  const s = 9 / zoom;
  return (
    <span
      role="presentation"
      className="border-primary bg-background absolute border"
      style={{ left: x - s / 2, top: y - s / 2, width: s, height: s, cursor: `${handle}-resize` }}
      onPointerDown={onDown}
    />
  );
}

function ItemView({
  it,
  detail,
  selected,
  editing,
  onEdit,
  onText,
  onDone,
}: {
  it: Item;
  detail: "full" | "summary" | "block";
  selected: boolean;
  editing: boolean;
  onEdit: () => void;
  onText: (t: string) => void;
  onDone: () => void;
}) {
  const box = { left: it.x, top: it.y, width: it.w, height: it.h } as const;
  const ring = selected ? "outline outline-2 outline-primary" : "";
  const text = it.type === "note" || it.type === "frame" || it.type === "shape" ? (it.text ?? "") : "";
  const body = editing ? (
    <textarea
      // biome-ignore lint/a11y/noAutofocus: the box was just opened for typing
      autoFocus
      className="size-full resize-none bg-transparent p-2 text-[13px] outline-none"
      defaultValue={text}
      onChange={(e) => onText(e.target.value)}
      onBlur={onDone}
      onPointerDown={(e) => e.stopPropagation()}
    />
  ) : detail === "block" ? null : (
    <div className="whitespace-pre-wrap break-words p-2 text-[13px]">{text}</div>
  );
  if (it.type === "frame")
    return (
      <div
        className={cn("absolute rounded-md border-2 border-dashed", TONE[it.tone ?? "plain"], "bg-opacity-30", ring)}
        style={box}
        role="option"
        id={itemDomId(it.item_id)}
        aria-selected={selected}
        tabIndex={-1}
        aria-label={itemLabel(it)}
        onDoubleClick={onEdit}
      >
        {detail !== "block" ? <div className="px-2 pt-1 text-[12px] font-medium opacity-70">{it.title}</div> : null}
        {body}
      </div>
    );
  if (it.type === "note")
    return (
      <div
        className={cn("absolute rounded-sm border shadow-sm", TONE[it.tone ?? "amber"], ring)}
        style={box}
        role="option"
        id={itemDomId(it.item_id)}
        aria-selected={selected}
        tabIndex={-1}
        aria-label={itemLabel(it)}
        onDoubleClick={onEdit}
      >
        {body}
      </div>
    );
  if (it.type === "shape")
    return (
      <div
        className={cn(
          "border-foreground/70 absolute flex items-center justify-center border-2",
          it.shape === "ellipse" && "rounded-full",
          it.shape === "diamond" && "rotate-45",
          ring,
        )}
        style={box}
        role="option"
        id={itemDomId(it.item_id)}
        aria-selected={selected}
        tabIndex={-1}
        aria-label={itemLabel(it)}
        onDoubleClick={onEdit}
      >
        {body}
      </div>
    );
  if (it.type === "image")
    return (
      <img
        alt={it.alt ?? it.title}
        src={it.src}
        draggable={false}
        className={cn("absolute select-none object-contain", ring)}
        style={box}
      />
    );
  if (isCard(it)) return <Card it={it} detail={detail} ring={ring} />;
  return null;
}
