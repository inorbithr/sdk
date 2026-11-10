// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 InOrbit d.o.o.
//
// The Grid… picker (iohr-mac window-layout-button.tsx): a small 6 by 4 grid under the layout button. Press on a cell,
// drag to another and let go to give the page that span; a plain click on one cell gives the smallest span around it.
// From the keyboard the arrows move the cursor, Enter sets the corner and Enter again applies, Escape clears the corner
// and then closes. A press outside closes it. The pure parts (the span, the cursor) are in frame/arrange.ts.

import { useEffect, useRef, useState } from "react";
import { GRID, moveCell, type Place, spanPlace } from "./arrange.ts";
import { cn } from "./cn.ts";

type Cell = { col: number; row: number };

const inside = (p: Place, c: Cell) => c.col >= p.c0 && c.col < p.c1 && c.row >= p.r0 && c.row < p.r1;

export function GridPicker({
  at,
  onPick,
  onClose,
}: {
  /** The button the picker hangs under; focus goes back to it on close. */
  at: HTMLElement;
  onPick: (p: Place) => void;
  onClose: () => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const [cursor, setCursor] = useState<Cell>({ col: 0, row: 0 });
  const [anchor, setAnchor] = useState<Cell | null>(null);
  const [pressed, setPressed] = useState(false);
  const r = at.getBoundingClientRect();
  const left = Math.max(8, Math.min(window.innerWidth - 8 - 188, r.right - 188));

  useEffect(() => {
    box.current?.focus();
    const away = (e: PointerEvent) => {
      if (!box.current?.contains(e.target as Node) && !at.contains(e.target as Node)) onClose();
    };
    document.addEventListener("pointerdown", away, true);
    return () => {
      document.removeEventListener("pointerdown", away, true);
      at.focus();
    };
  }, [at, onClose]);

  const span = anchor ? spanPlace(anchor, cursor) : null;
  const apply = (from: Cell, to: Cell) => {
    onPick(spanPlace(from, to));
    onClose();
  };

  return (
    <div
      ref={box}
      role="dialog"
      aria-label="Grid"
      tabIndex={-1}
      data-grid-picker
      style={{ position: "fixed", top: r.bottom + 4, left, zIndex: 60 }}
      className="bg-popover text-popover-foreground w-[188px] rounded-lg border p-2 shadow-lg outline-none"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          if (anchor) setAnchor(null);
          else onClose();
        } else if (e.key.startsWith("Arrow")) {
          e.preventDefault();
          setCursor((c) => moveCell(c, e.key));
        } else if (e.key === "Enter") {
          e.preventDefault();
          if (anchor) apply(anchor, cursor);
          else setAnchor(cursor);
        }
      }}
    >
      <div
        className="grid gap-1"
        style={{ gridTemplateColumns: `repeat(${GRID.cols}, 1fr)` }}
        onPointerUp={() => {
          if (pressed && anchor) apply(anchor, cursor);
          setPressed(false);
        }}
      >
        {Array.from({ length: GRID.rows * GRID.cols }, (_, i) => {
          const c = { col: i % GRID.cols, row: Math.floor(i / GRID.cols) };
          const on = span ? inside(span, c) : false;
          return (
            <button
              key={`${c.col}-${c.row}`}
              type="button"
              tabIndex={-1}
              aria-label={`Column ${c.col + 1}, row ${c.row + 1}`}
              data-cell={`${c.col},${c.row}`}
              onPointerDown={(e) => {
                e.preventDefault();
                setCursor(c);
                setAnchor(c);
                setPressed(true);
              }}
              onPointerEnter={() => pressed && setCursor(c)}
              className={cn(
                "h-5 rounded-[3px] border",
                on ? "bg-primary border-primary" : "bg-muted hover:bg-accent",
                !on && cursor.col === c.col && cursor.row === c.row && "ring-ring ring-2",
              )}
            />
          );
        })}
      </div>
      <p className="text-muted-foreground mt-2 text-[11px]">Drag over the cells, or Enter twice.</p>
    </div>
  );
}
