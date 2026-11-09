// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 InOrbit d.o.o.
//
// Moved from iohr-mac src/components/rail-flyout.tsx (origin/main d3ac8759) under ADR 0230: the shell frame layer
// lives here under Apache-2.0, and iohr-mac imports it (RFC 0217 C8 deletes its copy).
//
import { Pin } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { cn } from "./cn.ts";
import {
  type FlyoutEvent,
  type FlyoutState,
  initial,
  inSafeTriangle,
  type Pt,
  placeFlyout,
  step,
} from "./rail-flyout-core.ts";
import type { Badges } from "./sidebar-frame.ts";

// The collapsed rail's flyout (PRD 0106): the timing and keys live in src/lib/rail-flyout.ts; this
// hook feeds it events and one timer, and the view draws the popover beside the icon.

export type FlyoutRow = {
  key: string;
  label: string;
  status?: string;
  icon?: ReactNode;
  live?: boolean;
  dim?: boolean;
  away?: boolean;
  pinned?: boolean;
  badges?: Badges;
  isNew?: boolean;
  isUpdated?: boolean;
  /** A count the page shows on its row (Waiting on you, Approvals). */
  badge?: { n: number; live?: boolean; title?: string };
  onClick: () => void;
};

/** The "New" / "Updated" marker after a page's name: one look in the sidebar and in the rail's flyout. */
export function Marker({ isNew, isUpdated }: { isNew?: boolean; isUpdated?: boolean }) {
  if (!isNew && !isUpdated) return null;
  return (
    <span
      title={
        isNew
          ? "New: you have not opened it yet"
          : "Updated: it changed in the last update, and you have not opened it since"
      }
      className={cn(
        "rounded-full px-1.5 font-mono text-[9.5px] leading-4 tracking-wide uppercase",
        isNew ? "bg-primary text-primary-foreground" : "border-primary/50 text-primary border",
      )}
    >
      {isNew ? "New" : "Updated"}
    </span>
  );
}

/** One rail icon as the flyout sees it: its name (a link to its page) and the items under it. */
export type FlyoutSection = { id: string; label: string; status?: string; items: FlyoutRow[] };

// The value sits inside a double-quoted attribute selector. Without CSS.escape, a backslash and a quote are the two
// characters that can end or bend that string, so both are escaped (the backslash too, or `\"` would close it).
const attr = (s: string) => (typeof CSS !== "undefined" && CSS.escape ? CSS.escape(s) : s.replace(/["\\]/g, "\\$&"));

export function useRailFlyout(onGo: (id: string) => void, sections: Map<string, FlyoutSection>, active: boolean) {
  const [s, setS] = useState<FlyoutState>(initial);
  const ref = useRef(s);
  const goRef = useRef(onGo);
  goRef.current = onGo;
  const secRef = useRef(sections);
  secRef.current = sections;
  const left = useRef<Pt | null>(null);
  const lastIcon = useRef("");
  if (s.open) lastIcon.current = s.open;
  const box = useRef<HTMLDivElement>(null);

  const send = useCallback((e: FlyoutEvent) => {
    const r = step(ref.current, e, Date.now());
    ref.current = r.state;
    setS(r.state);
    if (r.go) goRef.current(r.go);
  }, []);

  // The one timer: whatever is pending (the hover delay or the close delay).
  const at = s.pending?.at;
  useEffect(() => {
    if (at === undefined) return;
    const id = window.setTimeout(() => send({ type: "tick" }), Math.max(0, at - Date.now()) + 2);
    return () => window.clearTimeout(id);
  }, [at, send]);

  // A close is pending: a pointer heading for the flyout holds it open (the safe triangle).
  const closing = s.pending?.op === "close";
  useEffect(() => {
    if (!closing) return;
    const move = (e: PointerEvent) => {
      const el = box.current;
      if (!el || !left.current) return;
      const r = el.getBoundingClientRect();
      send({ type: "pointerMove", safe: inSafeTriangle({ x: e.clientX, y: e.clientY }, left.current, r) });
    };
    window.addEventListener("pointermove", move);
    return () => window.removeEventListener("pointermove", move);
  }, [closing, send]);

  // Click elsewhere, Esc and the window losing focus close a flyout that is showing.
  const open = s.open;
  useEffect(() => {
    if (!open) return;
    const down = (e: MouseEvent) => {
      const t = e.target as HTMLElement | null;
      if (t?.closest("[data-rail-id],[data-rail-flyout]")) return;
      send({ type: "outsideClick" });
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && send({ type: "escape" });
    const blur = () => send({ type: "outsideClick" });
    window.addEventListener("mousedown", down);
    window.addEventListener("keydown", esc);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("mousedown", down);
      window.removeEventListener("keydown", esc);
      window.removeEventListener("blur", blur);
    };
  }, [open, send]);

  // Leaving the rail (the sidebar opens or hides) closes it; a section that vanished does too.
  useEffect(() => {
    if (!active) send({ type: "reset" });
  }, [active, send]);
  useEffect(() => {
    if (open && !sections.has(open)) send({ type: "reset" });
  }, [open, sections, send]);

  // Focus moves the machine asks for: once per request (focusSeq), read from the machine's current state.
  const focusSeq = s.focusSeq;
  useEffect(() => {
    const cur = ref.current;
    if (cur.focus === null || focusSeq === 0) return;
    const el =
      cur.focus === "icon"
        ? document.querySelector<HTMLElement>(`[data-rail-id="${attr(cur.open ?? lastIcon.current)}"]`)
        : document.querySelector<HTMLElement>(`[data-fly-i="${cur.focus}"]`);
    el?.focus();
  }, [focusSeq]);
  const icon = (id: string, hasItems: boolean, items: number) => ({
    "data-rail-id": id,
    "aria-haspopup": "menu" as const,
    "aria-expanded": s.open === id,
    onMouseEnter: () => send({ type: "iconEnter", id }),
    onMouseLeave: (e: React.MouseEvent) => {
      left.current = { x: e.clientX, y: e.clientY };
      send({ type: "iconLeave", id });
    },
    onClick: () => send({ type: "iconClick", id, hasItems }),
    onKeyDown: (e: React.KeyboardEvent) => {
      if (e.key === "ArrowRight" || (e.key === "Enter" && items > 0)) {
        e.preventDefault();
        send({ type: "keyOpen", id, items });
      }
    },
  });

  return { state: s, send, icon, box };
}

export function RailFlyout({
  section,
  anchor,
  send,
  box,
}: {
  section: FlyoutSection;
  anchor: HTMLElement | null;
  send: (e: FlyoutEvent) => void;
  box: React.RefObject<HTMLDivElement | null>;
}) {
  const [pos, setPos] = useState<{ left: number; top: number; maxHeight: number } | null>(null);
  const n = section.items.length;
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new section changes the flyout's rows, so it is placed again; its height is read from the DOM, not from section.
  useLayoutEffect(() => {
    const el = box.current;
    if (!el || !anchor) return;
    const place = () =>
      setPos(
        placeFlyout(anchor.getBoundingClientRect(), el.scrollHeight + 2, {
          w: window.innerWidth,
          h: window.innerHeight,
        }),
      );
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true); // the rail scrolls: the flyout follows its icon
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [anchor, box, section]);
  const name = (
    <button
      type="button"
      role="menuitem"
      data-fly-i={0}
      tabIndex={-1}
      onClick={() => send({ type: "iconClick", id: section.id, hasItems: false })}
      className="hover:bg-accent focus-visible:bg-accent flex min-h-8 w-full flex-col justify-center rounded-md px-2 py-1 text-left outline-none"
    >
      <span className="truncate text-[13px] font-medium">{section.label}</span>
      {section.status && <span className="text-muted-foreground truncate text-[11px] leading-4">{section.status}</span>}
    </button>
  );
  return (
    <div
      ref={box}
      role="menu"
      aria-label={section.label}
      data-rail-flyout={section.id}
      data-testid="rail-flyout"
      onMouseEnter={() => send({ type: "flyoutEnter" })}
      onMouseLeave={() => send({ type: "flyoutLeave" })}
      onKeyDown={(e) => {
        if (
          e.key === "ArrowDown" ||
          e.key === "ArrowUp" ||
          e.key === "ArrowLeft" ||
          e.key === "Home" ||
          e.key === "End"
        ) {
          e.preventDefault();
          send({ type: "key", key: e.key, items: n });
        } else if (e.key === "Tab") send({ type: "outsideClick" });
      }}
      style={{
        position: "fixed",
        left: pos?.left ?? (anchor ? anchor.getBoundingClientRect().right + 6 : 70),
        top: pos?.top ?? anchor?.getBoundingClientRect().top ?? 0,
        maxHeight: pos?.maxHeight,
      }}
      className="bg-popover text-popover-foreground animate-in fade-in-0 z-50 flex w-64 flex-col overflow-y-auto rounded-lg border p-1 text-[13px] shadow-lg duration-100"
    >
      {name}
      {n > 0 && (
        <>
          <hr className="bg-border mx-1 my-1 h-px shrink-0 border-0" />
          {section.items.map((r, i) => (
            <button
              key={r.key}
              type="button"
              role="menuitem"
              data-fly-i={i + 1}
              tabIndex={-1}
              onClick={() => {
                send({ type: "outsideClick" });
                r.onClick();
              }}
              className={cn(
                "hover:bg-accent focus-visible:bg-accent flex min-h-7 w-full items-center gap-2 rounded-md px-2 py-0.5 text-left outline-none [&>svg]:size-3.5 [&>svg]:shrink-0",
                (r.away || r.dim) && "text-muted-foreground",
                r.away && "opacity-60",
              )}
            >
              <span className="flex size-4 shrink-0 items-center justify-center">
                {r.live ? <span className="bg-success size-2 animate-pulse rounded-full" /> : r.icon}
              </span>
              <span className="min-w-0 flex-1">
                <span className="block truncate">{r.label}</span>
                {r.status && (
                  <span className="text-muted-foreground block truncate text-[11px] leading-4">{r.status}</span>
                )}
              </span>
              {r.pinned && <Pin className="text-muted-foreground size-3 shrink-0" />}
              {r.away && (
                <span role="img" aria-label="in maintenance" className="bg-warning size-1.5 shrink-0 rounded-full" />
              )}
              <Marker isNew={r.isNew} isUpdated={r.isUpdated} />
              {r.badge && (
                <span
                  className={cn(
                    "flex h-4 min-w-4 items-center justify-center gap-1 rounded-full px-1 font-mono text-[10px] tabular-nums",
                    r.badge.live ? "text-success" : "bg-foreground text-background",
                  )}
                >
                  {r.badge.live && <span className="bg-success size-1.5 animate-pulse rounded-full" />}
                  {r.badge.n}
                </span>
              )}
              {r.badges?.waiting ? (
                <Pill tone="bg-foreground text-background" title={`${r.badges.waiting} waiting on you`}>
                  {r.badges.waiting}
                </Pill>
              ) : null}
              {r.badges?.failing ? (
                <Pill tone="bg-destructive text-white" title={`${r.badges.failing} checks failing`}>
                  {r.badges.failing}
                </Pill>
              ) : null}
              {r.badges?.unread ? (
                <span title="new activity" className="bg-primary size-1.5 shrink-0 rounded-full" />
              ) : null}
            </button>
          ))}
        </>
      )}
    </div>
  );
}

function Pill({ tone, title, children }: { tone: string; title: string; children: ReactNode }) {
  return (
    <span
      title={title}
      className={cn(
        "flex h-4 min-w-4 items-center justify-center rounded-full px-1 font-mono text-[10px] tabular-nums",
        tone,
      )}
    >
      {children}
    </span>
  );
}
