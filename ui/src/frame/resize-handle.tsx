// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 InOrbit d.o.o.
//
// The one resize edge of the frame, folded from two copies in iohr-mac (origin/main d3ac8759) under ADR 0230: the
// sidebar's ResizeEdge (src/components/shell.tsx) and the dock's Resize (src/components/chat.tsx) were the same
// splitter with a different side and step. The edge sits on one side of its box; dragging measures the box from its
// other side, and the arrow that points away from the box makes it wider. iohr-mac imports it in RFC 0217 C8.
//
// It is an <hr>: a separator that takes focus and carries a value is the ARIA window splitter.

import type { PointerEvent as ReactPointerEvent } from "react";
import { cn } from "./cn.ts";
import { dragWidth, keyWidth } from "./resize-handle-core.ts";

export type ResizeHandleProps = {
  label: string;
  width: number;
  min: number;
  max: number;
  /** The box's edge the handle sits on. */
  edge: "left" | "right";
  /** Pixels per arrow press. */
  step: number;
  /** The width the pointer or the keys ask for; the app clamps it. */
  onWidth: (w: number) => void;
  /** Double-click: back to the first width. */
  onReset?: () => void;
  onStart?: () => void;
  onEnd?: () => void;
  className?: string;
};

export function ResizeHandle({
  label,
  width,
  min,
  max,
  edge,
  step,
  onWidth,
  onReset,
  onStart,
  onEnd,
  className,
}: ResizeHandleProps) {
  const drag = (e: ReactPointerEvent<HTMLHRElement>) => {
    e.preventDefault();
    const el = e.currentTarget;
    el.setPointerCapture(e.pointerId);
    const r = el.parentElement?.getBoundingClientRect();
    const box = { left: r?.left ?? 0, right: r?.right ?? window.innerWidth };
    onStart?.();
    const move = (m: PointerEvent) => onWidth(dragWidth(m.clientX, box, edge));
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
      onEnd?.();
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
  };
  return (
    <hr
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={width}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      onPointerDown={drag}
      onDoubleClick={onReset}
      onKeyDown={(e) => {
        const w = keyWidth(e.key, width, edge, step);
        if (w === null) return;
        e.preventDefault();
        onWidth(w);
      }}
      className={cn(
        "absolute inset-y-0 z-10 m-0 h-auto w-1.5 cursor-col-resize border-0 outline-none transition-colors",
        edge === "left" ? "left-0" : "right-0",
        className,
      )}
    />
  );
}
