// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 InOrbit d.o.o.
//
// The arithmetic of the frame's one resize edge (frame/resize-handle.tsx), folded under ADR 0230 from iohr-mac's
// sidebar ResizeEdge (src/components/shell.tsx) and dock Resize (src/components/chat.tsx), origin/main d3ac8759.
// Pure, so node tests it.

/** The width a key asks for: the arrow away from the box grows it, the other shrinks it; other keys ask nothing. */
export function keyWidth(key: string, width: number, edge: "left" | "right", step: number): number | null {
  if (key !== "ArrowLeft" && key !== "ArrowRight") return null;
  return width + ((key === "ArrowLeft") === (edge === "left") ? step : -step);
}

/** The width the pointer at `x` gives a box spanning `left` to `right`, measured from the side away from the edge. */
export const dragWidth = (x: number, box: { left: number; right: number }, edge: "left" | "right") =>
  edge === "left" ? box.right - x : x - box.left;
