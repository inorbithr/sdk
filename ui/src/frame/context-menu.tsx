// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 InOrbit d.o.o.
//
// Moved from iohr-mac src/components/context-menu.tsx (origin/main d3ac8759) under ADR 0230: the shell frame layer
// lives here under Apache-2.0, and iohr-mac imports it (RFC 0217 C8 deletes its copy). Changes from the Mac copy:
// written without comma operators and index keys (Biome's rules here), a row's click is read on the menu itself
// (which owns the keys through aria-activedescendant), and `trigger` lets a button toggle the menu it opened.
//
// The one context menu (PRD "Enterprise context menu"). A surface calls openContextMenu(at, entries) or
// showContextMenu(event, entries); the model and the pure rules (groups, glyphs, arrows, type-ahead, flipping at the
// edges) are in context-menu-core.ts.
//
// Looks: the theme's own tokens (popover, border, accent, destructive, muted), light and dark; a blurred translucent
// surface, soft shadow, rounded corners; leading icons from lucide (the app's set); shortcuts as Mac glyphs in muted keycaps.
// Keys: arrows, Home/End, Enter/Space, → opens a submenu and ← or Esc closes it, type-ahead, Esc closes, Tab closes;
// a search row appears for a long menu. ARIA: menu, menuitem, menuitemcheckbox, aria-activedescendant, aria-haspopup.
import {
  AppWindow,
  ArrowDown,
  ArrowDownToLine,
  ArrowLeftToLine,
  ArrowRightToLine,
  ArrowUp,
  ArrowUpToLine,
  BoxSelect,
  Check,
  ChevronLeft,
  ChevronRight,
  ClipboardCopy,
  ClipboardPaste,
  Columns3,
  Command,
  Copy,
  CopyPlus,
  Crop,
  Download,
  Eye,
  EyeOff,
  FileCode2,
  FlipHorizontal2,
  FlipVertical2,
  Frame,
  Grid3x3,
  Group,
  Image as ImageIcon,
  Keyboard,
  Layers,
  LayoutGrid,
  LayoutPanelLeft,
  Library,
  Link2,
  ListChecks,
  Lock,
  LockOpen,
  type LucideIcon,
  Magnet,
  Maximize2,
  MessageSquare,
  MousePointer2,
  Orbit,
  Palette,
  PanelRight,
  Pin,
  PinOff,
  Puzzle,
  Redo2,
  RotateCcw,
  Route,
  Scissors,
  Search,
  SlidersHorizontal,
  Sparkles,
  Square,
  Terminal,
  Trash2,
  Type,
  Undo2,
  Ungroup,
  Wrench,
} from "lucide-react";
import {
  type CSSProperties,
  type KeyboardEvent,
  type MouseEvent,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { createPortal } from "react-dom";
import { cn } from "./cn.ts";
import {
  countActions,
  first,
  isAction,
  isHeading,
  isOff,
  isSep,
  keyedCaps,
  last,
  type MenuAction,
  type MenuEntry,
  type Placed,
  place,
  placeSub,
  reasonOf,
  rowKey,
  search,
  step,
  typeAhead,
} from "./context-menu-core.ts";

// ---------------------------------------------------------------- icons

export const ICONS: Record<string, LucideIcon> = {
  ask: Sparkles,
  chat: MessageSquare,
  copy: Copy,
  "copy-as": ClipboardCopy,
  cut: Scissors,
  paste: ClipboardPaste,
  image: ImageIcon,
  code: FileCode2,
  "select-all": BoxSelect,
  select: MousePointer2,
  lock: Lock,
  unlock: LockOpen,
  grid: Grid3x3,
  snap: Magnet,
  zen: Maximize2,
  view: Eye,
  hide: EyeOff,
  properties: SlidersHorizontal,
  delete: Trash2,
  group: Group,
  ungroup: Ungroup,
  layers: Layers,
  front: ArrowUpToLine,
  back: ArrowDownToLine,
  forward: ArrowUp,
  backward: ArrowDown,
  "flip-h": FlipHorizontal2,
  "flip-v": FlipVertical2,
  link: Link2,
  library: Library,
  styles: Palette,
  duplicate: CopyPlus,
  frame: Frame,
  crop: Crop,
  text: Type,
  pin: Pin,
  unpin: PinOff,
  panel: PanelRight,
  window: AppWindow,
  download: Download,
  search: Search,
  command: Command,
  keyboard: Keyboard,
  undo: Undo2,
  redo: Redo2,
  route: Route,
  tasks: ListChecks,
  terminal: Terminal,
  // Added here for the extension panels (RFC 0217): an extension, and stopping one.
  extension: Puzzle,
  stop: Square,
  // Added here for the layout, Arrange, Orbits and Maintenance menus (RFC 0217, slice 2).
  layout: LayoutGrid,
  "layout-pair": LayoutPanelLeft,
  columns: Columns3,
  orbit: Orbit,
  wrench: Wrench,
  reset: RotateCcw,
  left: ChevronLeft,
  right: ChevronRight,
  "to-start": ArrowLeftToLine,
  "to-end": ArrowRightToLine,
};

// ---------------------------------------------------------------- the store: one menu open at a time

export type OpenOptions = {
  /** A line above the items (the thing the menu is about). */
  label?: string;
  /** Force the search row on or off; by default it shows from 16 actions. */
  search?: boolean;
  /** Where focus goes back to when it closes (default: what had it). */
  restore?: HTMLElement | null;
  /** The button that opened it: a press there leaves the menu to the button's own click, which closes it. */
  trigger?: HTMLElement | null;
  /** Runs once when the menu is gone, however it closed. */
  onClose?: () => void;
};
type Point = { x: number; y: number };
type Req = { id: number; at: Point; entries: MenuEntry[]; opts: OpenOptions; restore: HTMLElement | null };

let current: Req | null = null;
let seq = 0;
const subs = new Set<() => void>();
const emit = () => {
  for (const f of subs) f();
};
const subscribe = (f: () => void) => {
  subs.add(f);
  return () => {
    subs.delete(f);
  };
};

export function openContextMenu(at: Point, entries: MenuEntry[], opts: OpenOptions = {}): number {
  const prev = current;
  current = null;
  prev?.opts.onClose?.();
  const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  seq += 1;
  current = { id: seq, at, entries, opts, restore: opts.restore !== undefined ? opts.restore : active };
  emit();
  return current.id;
}
export function closeContextMenu(id?: number) {
  if (!current || (id !== undefined && current.id !== id)) return;
  const gone = current;
  current = null;
  emit();
  gone.opts.onClose?.();
}
export const contextMenuOpen = () => current !== null;

/** From a mouse or React event: stop the browser's menu and open ours at the pointer. */
export function showContextMenu(
  e: { clientX: number; clientY: number; preventDefault: () => void; stopPropagation?: () => void },
  entries: MenuEntry[],
  opts?: OpenOptions,
) {
  e.preventDefault();
  e.stopPropagation?.();
  openContextMenu({ x: e.clientX, y: e.clientY }, entries, opts);
}

// ---------------------------------------------------------------- the host

/** Mounted once per window (main.tsx). */
export function ContextMenuHost() {
  const req = useSyncExternalStore(
    subscribe,
    () => current,
    () => null,
  );
  if (!req) return null;
  return createPortal(<Menu key={req.id} req={req} />, document.body);
}

const reduced = () => typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

function Menu({ req }: { req: Req }) {
  const [leaving, setLeaving] = useState(false);
  const done = useRef(false);
  const dismiss = useCallback(
    (run?: () => void) => {
      if (done.current) return;
      done.current = true;
      setLeaving(true);
      req.restore?.focus?.({ preventScroll: true });
      run?.();
      if (reduced()) closeContextMenu(req.id);
      else setTimeout(() => closeContextMenu(req.id), 90);
    },
    [req],
  );
  const drop = useCallback(() => {
    if (done.current) return;
    done.current = true;
    closeContextMenu(req.id);
  }, [req.id]);

  useEffect(() => {
    const trigger = req.opts.trigger;
    const out = (e: Event) => {
      if (!(e.target instanceof Node)) return drop();
      if (trigger?.contains(e.target)) return;
      if (!(e.target instanceof Element) || !e.target.closest("[data-ctx-menu]")) drop();
    };
    const shut = () => drop();
    window.addEventListener("pointerdown", out, true);
    window.addEventListener("scroll", out, true);
    window.addEventListener("blur", shut);
    window.addEventListener("resize", shut);
    return () => {
      window.removeEventListener("pointerdown", out, true);
      window.removeEventListener("scroll", out, true);
      window.removeEventListener("blur", shut);
      window.removeEventListener("resize", shut);
    };
  }, [drop, req.opts.trigger]);

  const anchor = useMemo(() => ({ at: req.at }), [req.at]);
  return (
    <Panel
      entries={req.entries}
      anchor={anchor}
      depth={0}
      label={req.opts.label}
      searchable={req.opts.search ?? countActions(req.entries) >= 16}
      leaving={leaving}
      onChoose={(a) => dismiss(a.run)}
      onClose={() => dismiss()}
      focusToken={1}
    />
  );
}

// ---------------------------------------------------------------- a panel (the menu, or one submenu)

type Rect = { left: number; right: number; top: number; bottom: number };
type Anchor = { at: Point } | { row: Rect };

function Panel({
  entries,
  anchor,
  depth,
  label,
  searchable,
  leaving,
  focusToken,
  onChoose,
  onClose,
  onBack,
}: {
  entries: MenuEntry[];
  anchor: Anchor;
  depth: number;
  label?: string;
  searchable?: boolean;
  leaving?: boolean;
  /** Raised each time the panel should take the keyboard (it opens focused when above 0). */
  focusToken: number;
  onChoose: (a: MenuAction) => void;
  onClose: () => void;
  onBack?: () => void;
}) {
  const uid = useId();
  const el = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(-1);
  const [sub, setSub] = useState<{ i: number; rect: Rect; token: number } | null>(null);
  const [pos, setPos] = useState<Placed | null>(null);
  const typed = useRef<{ buf: string; t: ReturnType<typeof setTimeout> | 0 }>({ buf: "", t: 0 });
  const hover = useRef<ReturnType<typeof setTimeout> | 0>(0);

  const results = useMemo(() => (query ? search(entries, query) : null), [entries, query]);
  const rows: MenuEntry[] = useMemo(
    () =>
      results
        ? results.map((r) => ({ ...r.action, hint: r.path.length ? r.path.join(" › ") : r.action.hint }))
        : entries,
    [results, entries],
  );
  // The rows when the keyboard arrives, without making that effect run again on every new list.
  const rowsNow = useRef(rows);
  rowsNow.current = rows;

  // Measure, then place: flips at the right and bottom edges, kept inside the window. Placed again whenever the
  // panel's size changes (a search narrowing the rows), before that frame is painted.
  useLayoutEffect(() => {
    const n = el.current;
    if (!n) return;
    const measure = () => {
      const size = { w: n.offsetWidth, h: n.scrollHeight + 2 };
      const view = { w: window.innerWidth, h: window.innerHeight };
      setPos("at" in anchor ? place(anchor.at, size, view) : placeSub(anchor.row, size, view));
    };
    measure();
    const seen = new ResizeObserver(measure);
    seen.observe(n);
    return () => seen.disconnect();
  }, [anchor]);

  useEffect(() => {
    if (focusToken <= 0) return;
    el.current?.focus({ preventScroll: true });
    // A surface under the menu (the canvas engine) may take the focus back as it opens: ask again once it has settled.
    if (depth === 0) {
      const timers = [0, 60, 200].map((ms) =>
        setTimeout(() => {
          const a = document.activeElement;
          if (el.current && !a?.closest?.("[data-ctx-menu]")) el.current.focus({ preventScroll: true });
        }, ms),
      );
      return () => {
        for (const t of timers) clearTimeout(t);
      };
    }
    setActive((a) => (a < 0 ? first(rowsNow.current) : a));
  }, [focusToken, depth]);
  useEffect(
    () => () => {
      if (hover.current) clearTimeout(hover.current);
    },
    [],
  );
  useEffect(() => {
    // Keep the active row in view while the arrows move through a scrolling panel.
    el.current?.querySelector(`[data-i="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const openSub = (i: number, withFocus: boolean) => {
    const row = el.current?.querySelector(`[data-i="${i}"]`)?.getBoundingClientRect();
    if (!row) return;
    const rect = { left: row.left, right: row.right, top: row.top, bottom: row.bottom };
    setSub((s) => ({ i, rect, token: withFocus ? (s?.token ?? 0) + 1 : s?.i === i ? s.token : 0 }));
  };

  const pick = (i: number) => {
    const r = rows[i];
    if (!r || !isAction(r)) return;
    if (r.items) {
      openSub(i, true);
      return;
    }
    if (isOff(r)) return;
    onChoose(r);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const inField = e.target === input.current;
    const mod = e.metaKey || e.ctrlKey || e.altKey;
    // Nothing leaks to the page under the menu (the canvas would move its selection on an arrow).
    if (!mod) e.stopPropagation();
    switch (e.key) {
      case "ArrowDown":
      case "ArrowUp": {
        e.preventDefault();
        const dir = e.key === "ArrowDown" ? 1 : -1;
        setActive((a) => step(rows, a, dir));
        return;
      }
      case "Home":
        if (inField) return;
        e.preventDefault();
        setActive(first(rows));
        return;
      case "End":
        if (inField) return;
        e.preventDefault();
        setActive(last(rows));
        return;
      case "ArrowRight": {
        if (inField) return;
        const r = rows[active];
        if (r && isAction(r) && r.items) {
          e.preventDefault();
          openSub(active, true);
        }
        return;
      }
      case "ArrowLeft":
        if (inField || !onBack) return;
        e.preventDefault();
        onBack();
        return;
      case "Enter":
        e.preventDefault();
        pick(active);
        return;
      case " ":
        if (inField) return;
        e.preventDefault();
        pick(active);
        return;
      case "Escape":
        e.preventDefault();
        // A submenu open: Esc closes it first.
        if (sub) setSub(null);
        else if (onBack) onBack();
        else onClose();
        return;
      case "Tab":
        e.preventDefault();
        onClose();
        return;
    }
    if (e.key.length !== 1 || mod || inField) return;
    if (searchable) {
      // Typing in a long menu searches it.
      e.preventDefault();
      setQuery((q) => q + e.key);
      setActive(-1);
      requestAnimationFrame(() => input.current?.focus());
      return;
    }
    const t = typed.current;
    t.buf += e.key;
    if (t.t) clearTimeout(t.t);
    t.t = setTimeout(() => {
      t.buf = "";
    }, 700);
    const i = typeAhead(rows, t.buf, active);
    if (i >= 0) setActive(i);
  };

  // A row's click, read here: the menu owns the keys (aria-activedescendant), so the rows themselves take no focus.
  const onClick = (e: MouseEvent) => {
    const row = e.target instanceof Element ? e.target.closest<HTMLElement>("[data-i]") : null;
    if (row && el.current?.contains(row)) pick(Number(row.dataset.i));
  };

  const hoverRow = (i: number, r: MenuAction) => {
    if (hover.current) clearTimeout(hover.current);
    if (active !== i) setActive(i);
    if (r.items) {
      if (sub) openSub(i, false);
      else hover.current = setTimeout(() => openSub(i, false), 140);
    } else if (sub) {
      setSub(null);
    }
  };

  const id = (i: number) => `${uid}-${i}`;
  const subEntry = sub ? rows[sub.i] : null;
  const dir = pos?.flipX ? "right" : "left";
  const style: CSSProperties = pos
    ? { left: pos.left, top: pos.top, maxHeight: pos.maxH, transformOrigin: `${dir} ${pos.flipY ? "bottom" : "top"}` }
    : { left: 0, top: 0, visibility: "hidden" };

  return (
    <>
      <div
        ref={el}
        role="menu"
        aria-orientation="vertical"
        aria-label={label || "Menu"}
        aria-activedescendant={active >= 0 ? id(active) : undefined}
        tabIndex={-1}
        data-ctx-menu={depth}
        data-leaving={leaving ? "" : undefined}
        onKeyDown={onKeyDown}
        onClick={onClick}
        // A press on the menu must not reach the page under it: the canvas engine closes its own menu on any outside press.
        onPointerDown={(e) => e.stopPropagation()}
        onMouseDown={(e) => {
          if (e.target !== input.current) e.preventDefault();
        }}
        onContextMenu={(e) => e.preventDefault()}
        style={style}
        className="iohr-ctx text-popover-foreground fixed z-[1000] w-max max-w-[340px] min-w-[216px] overflow-x-hidden overflow-y-auto overscroll-contain rounded-xl border p-1.5 text-[13px] outline-none select-none"
      >
        {searchable && depth === 0 && (
          <div className="border-border mb-1 flex items-center gap-2 border-b px-2 pt-0.5 pb-1.5">
            <Search className="text-muted-foreground size-3.5 shrink-0" />
            <input
              ref={input}
              value={query}
              onChange={(e) => {
                setQuery(e.target.value);
                setActive(-1);
              }}
              placeholder="Search actions"
              aria-label="Search actions"
              spellCheck={false}
              autoComplete="off"
              className="placeholder:text-muted-foreground min-w-0 flex-1 bg-transparent text-[13px] outline-none"
            />
          </div>
        )}
        {label && !query && (
          <div className="text-muted-foreground max-w-[300px] truncate px-2 pt-0.5 pb-1.5 text-[11px] font-medium tracking-wide">
            {label}
          </div>
        )}
        {results && results.length === 0 && (
          <div className="text-muted-foreground px-2 py-3 text-center text-[12px]">No action matches “{query}”.</div>
        )}
        {rows.map((r, i) => {
          if (isSep(r)) return <hr key={rowKey(rows, i)} className="bg-border mx-1 my-1.5 h-px border-0" />;
          if (isHeading(r))
            return (
              <div
                key={rowKey(rows, i)}
                role="presentation"
                className="text-muted-foreground px-2 pt-1.5 pb-1 text-[10.5px] font-medium tracking-[0.08em] uppercase"
              >
                {r.heading}
              </div>
            );
          const off = isOff(r);
          const isOn = i === active;
          const Icon = r.icon ? ICONS[r.icon] : undefined;
          const toggle = r.checked !== undefined;
          const reason = reasonOf(r);
          return (
            <div
              key={rowKey(rows, i)}
              id={id(i)}
              data-i={i}
              data-id={r.id}
              {...roleOf(r, sub?.i === i)}
              tabIndex={-1}
              aria-disabled={off || undefined}
              aria-description={reason || undefined}
              title={reason || undefined}
              onPointerMove={() => hoverRow(i, r)}
              className={cn(
                "flex h-7 items-center gap-2.5 rounded-md px-2 transition-colors",
                off ? "text-muted-foreground/60 cursor-default" : "cursor-default",
                !off && r.danger ? "text-destructive" : "",
                isOn && !off && (r.danger ? "bg-destructive/10" : "bg-accent text-accent-foreground"),
                isOn && off && "bg-accent/50",
                sub?.i === i && !isOn && "bg-accent/60",
              )}
            >
              <span className="grid size-4 shrink-0 place-items-center">
                {Icon && (
                  <Icon
                    className={cn(
                      "size-4",
                      off ? "opacity-60" : r.danger ? "" : isOn ? "text-accent-foreground" : "text-muted-foreground",
                    )}
                  />
                )}
              </span>
              <span className="min-w-0 flex-1 truncate">{r.label}</span>
              {r.hint && <span className="text-muted-foreground max-w-[120px] truncate text-[11.5px]">{r.hint}</span>}
              {toggle && r.checked && <Check className="size-3.5 shrink-0" aria-hidden />}
              {r.keys && <Keys keys={r.keys} dim={off} />}
              {r.items && <ChevronRight className="text-muted-foreground -mr-0.5 size-3.5 shrink-0" aria-hidden />}
            </div>
          );
        })}
      </div>
      {sub && subEntry && isAction(subEntry) && subEntry.items && (
        <Panel
          entries={subEntry.items}
          anchor={{ row: sub.rect }}
          depth={depth + 1}
          label={subEntry.label}
          leaving={leaving}
          focusToken={sub.token}
          onChoose={onChoose}
          onClose={onClose}
          onBack={() => {
            setSub(null);
            el.current?.focus({ preventScroll: true });
          }}
        />
      )}
    </>
  );
}

/** A row's role and the states that go with it: a toggle is a menuitemcheckbox that says whether it is on, a row
 *  with a submenu is a menuitem that says the submenu is open, any other row is a plain menuitem. */
type RowRole =
  | { role: "menuitemcheckbox"; "aria-checked": boolean }
  | { role: "menuitem"; "aria-haspopup": "menu"; "aria-expanded": boolean }
  | { role: "menuitem" };
function roleOf(r: MenuAction, open: boolean): RowRole {
  if (r.checked !== undefined) return { role: "menuitemcheckbox", "aria-checked": r.checked };
  if (r.items) return { role: "menuitem", "aria-haspopup": "menu", "aria-expanded": open };
  return { role: "menuitem" };
}

/** Shortcut glyphs, one muted keycap each: ⇧ ⌘ C. */
export function Keys({ keys, dim }: { keys: string; dim?: boolean }) {
  return (
    <span aria-hidden className={cn("ml-3 flex shrink-0 items-center gap-0.5 font-sans", dim && "opacity-50")}>
      {keyedCaps(keys).map(({ cap, key }) => (
        <kbd
          key={key}
          className="bg-foreground/[0.06] text-muted-foreground ring-border grid h-[18px] min-w-[18px] place-items-center rounded-[5px] px-1 font-sans text-[11px] leading-none ring-1 ring-inset"
        >
          {cap}
        </kbd>
      ))}
    </span>
  );
}
