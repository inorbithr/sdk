// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 InOrbit d.o.o.
//
// The right dock, moved from iohr-mac (origin/main d3ac8759) under ADR 0230: a box beside the page, raised on the
// sidebar's ground like the page itself, with its resize edge on the side that faces the page so the page gives or
// takes the width (src/components/chat.tsx placeOf and Resize), between the limits of frame/dock-frame.ts. The frame
// passes it in its `dock` slot; what it holds belongs to the app. iohr-mac imports it in RFC 0217 C8.

import { X } from "lucide-react";
import { type ReactNode, useState } from "react";
import { cn } from "./cn.ts";
import { clampPanel, FIRST_PANEL, MIN_PANEL, maxPanel } from "./dock-frame.ts";
import { useViewportWidth } from "./frame.tsx";
import { ResizeHandle } from "./resize-handle.tsx";

export type DockProps = {
  /** The box's name, shown in its header and given to the region. */
  title: string;
  /** The stored width; the dock draws it clamped to the window. */
  width: number;
  /** A width the pointer or the arrow keys ask for; the app keeps it (clamped with clampPanel). */
  onWidth: (w: number) => void;
  onClose: () => void;
  children: ReactNode;
};

export function Dock({ title, width, onWidth, onClose, children }: DockProps) {
  const win = useViewportWidth();
  const [resizing, setResizing] = useState(false);
  const shown = clampPanel(width, win);
  return (
    <aside
      aria-label={title}
      style={{ width: shown }}
      className={cn("relative flex shrink-0 flex-col py-2 pr-2", resizing && "select-none")}
    >
      <ResizeHandle
        label={`Resize ${title}`}
        width={shown}
        min={MIN_PANEL}
        max={maxPanel(win)}
        edge="left"
        step={24}
        onWidth={onWidth}
        onReset={() => onWidth(FIRST_PANEL)}
        onStart={() => setResizing(true)}
        onEnd={() => setResizing(false)}
        className="inset-y-2 hover:bg-ring/30 focus-visible:bg-ring/50"
      />
      <section className="bg-background text-foreground ml-1.5 flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border shadow-sm">
        <header className="flex h-11 shrink-0 items-center gap-2 border-b px-4 text-sm">
          <h2 className="min-w-0 flex-1 truncate font-medium">{title}</h2>
          <button
            type="button"
            title={`Close ${title}`}
            aria-label={`Close ${title}`}
            onClick={onClose}
            className="text-muted-foreground hover:bg-accent hover:text-foreground flex size-7 items-center justify-center rounded-md"
          >
            <X className="size-4" />
          </button>
        </header>
        <div className="min-h-0 flex-1 overflow-auto">{children}</div>
      </section>
    </aside>
  );
}
