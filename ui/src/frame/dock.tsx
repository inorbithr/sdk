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
  /** Which side of the page the dock is on (default the right); its resize edge is on the side that faces the page. */
  side?: "left" | "right";
  /** The dock fills the cells it was given (a layout put it beside the page) instead of keeping its own width. */
  fill?: boolean;
  /** Buttons in the header before Close (the Arrange button). */
  headerActions?: ReactNode;
  children: ReactNode;
};

export function Dock({
  title,
  width,
  onWidth,
  onClose,
  side = "right",
  fill = false,
  headerActions,
  children,
}: DockProps) {
  const win = useViewportWidth();
  const [resizing, setResizing] = useState(false);
  const shown = clampPanel(width, win);
  return (
    <aside
      aria-label={title}
      style={fill ? undefined : { width: shown }}
      className={cn(
        "relative flex flex-col py-2",
        fill ? "min-w-0 flex-1" : "shrink-0",
        side === "right" ? "pr-2" : "pl-0",
        resizing && "select-none",
      )}
    >
      {!fill && (
        <ResizeHandle
          label={`Resize ${title}`}
          width={shown}
          min={MIN_PANEL}
          max={maxPanel(win)}
          edge={side === "right" ? "left" : "right"}
          step={24}
          onWidth={onWidth}
          onReset={() => onWidth(FIRST_PANEL)}
          onStart={() => setResizing(true)}
          onEnd={() => setResizing(false)}
          className="inset-y-2 hover:bg-ring/30 focus-visible:bg-ring/50"
        />
      )}
      <section
        className={cn(
          "bg-background text-foreground flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border shadow-sm",
          side === "right" ? "ml-1.5" : "mr-2",
        )}
      >
        <header className="flex h-11 shrink-0 items-center gap-2 border-b px-4 text-sm">
          <h2 className="min-w-0 flex-1 truncate font-medium">{title}</h2>
          {headerActions}
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
