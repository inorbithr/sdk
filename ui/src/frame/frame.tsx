// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 InOrbit d.o.o.
//
// Moved from iohr-mac src/components/shell.tsx (the layout) and src/lib/sidebar-store.ts
// (useViewportWidth) (origin/main d3ac8759) under ADR 0230: the shell frame layer lives here under
// Apache-2.0, and iohr-mac imports it (RFC 0217 C8 deletes its copy). The frame draws the window: the
// sidebar column (title strip, body, footer, resize edge) and the page inset (header, actions, body),
// with a dock slot beside the page (RFC 0217 C7). What goes in each slot belongs to the app; the frame
// owns no state, so each app keeps the sidebar where it likes (a file in the Mac app, the browser here).

import { PanelLeftClose, PanelLeftOpen } from "lucide-react";
import { type ReactNode, useState, useSyncExternalStore } from "react";
import { type Arrangement, arranged, describe, initial, layoutOff, type Place } from "./arrange.ts";
import { cn } from "./cn.ts";
import { ResizeHandle } from "./resize-handle.tsx";
import { keysOf } from "./shortcuts.ts";
import {
  columnWidth,
  DEFAULT_WIDTH,
  effectiveMode,
  MAX_WIDTH,
  MIN_WIDTH,
  type SidebarFrame,
  type SidebarMode,
  setMode,
  setWidth,
  toggleCollapsed,
} from "./sidebar-frame.ts";

/** Changes the sidebar's stored state; the app decides where it is kept. */
export type UpdateSidebar = (f: <S extends SidebarFrame>(s: S) => S) => void;

/** The window's inner width, live. */
export function useViewportWidth() {
  return useSyncExternalStore(
    (f) => {
      window.addEventListener("resize", f);
      return () => window.removeEventListener("resize", f);
    },
    () => window.innerWidth,
    () => 1280,
  );
}

/** What is drawn now: the stored mode, with a narrow window folding "expanded" to the rail. */
export function useFrameMode(sidebar: SidebarFrame): SidebarMode {
  return effectiveMode(sidebar.mode, useViewportWidth());
}

/** Below this window width the arrangement is not applied: the page fills the area, as on a phone. */
export const ARRANGE_MIN_WIDTH = 900;

/** What a dock is told about where it sits, so it can be drawn for it. */
export type DockSlot = { side: "left" | "right"; fill: boolean };

const cell = (p: Place) => ({ gridColumn: `${p.c0 + 1} / ${p.c1 + 1}`, gridRow: `${p.r0 + 1} / ${p.r1 + 1}` });

export type FrameProps = {
  sidebar: SidebarFrame;
  update: UpdateSidebar;
  /** Under the title strip: a switcher, a search. Told whether the column is the rail. */
  sidebarTop?: (rail: boolean) => ReactNode;
  /** The sidebar's lists. */
  sidebarBody: (rail: boolean) => ReactNode;
  /** The sidebar's last row: settings, the signed-in person. */
  sidebarFooter?: (rail: boolean) => ReactNode;
  /** The page header's left part: where you are. */
  title: ReactNode;
  /** The page header's right part: the page's buttons and menus. */
  actions?: ReactNode;
  /** Room for a native window's traffic lights when the sidebar is hidden (the Mac app). */
  trafficLights?: boolean;
  /** The dock beside the page (frame/dock.tsx, RFC 0217 C7); the app decides what it holds. A function is told the side
   *  and whether it fills cells of its own (frame/arrange.ts), so it can be drawn for them. */
  dock?: ReactNode | ((slot: DockSlot) => ReactNode);
  /** Where the page and the dock sit (frame/arrange.ts); left out, the page fills the area with the dock on its right. */
  arrangement?: Arrangement;
  /** With an arrangement: the way back to the page filling the area, shown while it does not. */
  onRestore?: () => void;
  children: ReactNode;
};

const iconButton =
  "text-muted-foreground hover:bg-sidebar-accent hover:text-foreground ring-sidebar-ring flex size-8 shrink-0 items-center justify-center rounded-md outline-none focus-visible:ring-2";

/** Puts focus back on the toggle after a keyboard press: it moves between the title strip and the rail column. */
const refocusToggle = (detail: number) => {
  if (detail === 0)
    requestAnimationFrame(() =>
      requestAnimationFrame(() => document.querySelector<HTMLElement>("[data-sidebar-toggle]")?.focus()),
    );
};

export function Frame({
  sidebar,
  update,
  sidebarTop,
  sidebarBody,
  sidebarFooter,
  title,
  actions,
  trafficLights,
  dock,
  arrangement,
  onRestore,
  children,
}: FrameProps) {
  const mode = useFrameMode(sidebar);
  const wide = useViewportWidth() >= ARRANGE_MIN_WIDTH;
  const arr = arrangement && wide ? arrangement : initial();
  const dockFirst = arr.order[0] === "dock";
  const dockNode =
    typeof dock === "function" ? dock({ side: dockFirst ? "left" : "right", fill: arr.dock !== null }) : dock;
  const rail = mode === "rail";
  const hidden = mode === "hidden";
  const [resizing, setResizing] = useState(false);
  const toggle = (
    <button
      type="button"
      title={rail ? "Expand sidebar" : "Collapse to icons"}
      aria-label={rail ? "Expand sidebar" : "Collapse sidebar"}
      data-sidebar-toggle
      onClick={(e) => {
        update(toggleCollapsed);
        refocusToggle(e.detail);
      }}
      className={iconButton}
    >
      {rail ? <PanelLeftOpen className="size-4" /> : <PanelLeftClose className="size-4" />}
    </button>
  );
  return (
    <div className="bg-sidebar text-sidebar-foreground flex h-full">
      {/* The column animates its width; its contents never reflow mid-flight (overflow clipped). */}
      <aside
        aria-hidden={hidden}
        inert={hidden}
        style={{ width: columnWidth(mode, sidebar.width) }}
        className={cn(
          "relative flex shrink-0 flex-col overflow-hidden",
          !resizing && "transition-[width] duration-200 ease-out motion-reduce:transition-none",
        )}
      >
        {/* Expanded: the title strip holds the toggle on the right. Rail: the strip is empty and the toggle stacks below. */}
        <div data-tauri-drag-region="deep" className="flex h-[52px] shrink-0 items-center justify-end gap-1 px-2">
          {!rail && toggle}
        </div>
        <div className={cn("flex pb-2", rail ? "flex-col items-center gap-1 px-2" : "px-2")}>
          <div className={cn("min-w-0", !rail && "flex-1")}>{sidebarTop?.(rail)}</div>
          {rail && toggle}
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto">{sidebarBody(rail)}</div>
        {sidebarFooter && <div className="border-sidebar-border border-t p-2">{sidebarFooter(rail)}</div>}
        {/* The expanded sidebar's right edge: drag to resize (200 to 360 px), arrows nudge, double-click resets. */}
        {!rail && !hidden && (
          <ResizeHandle
            label="Resize sidebar"
            width={sidebar.width}
            min={MIN_WIDTH}
            max={MAX_WIDTH}
            edge="right"
            step={16}
            onWidth={(w) => update((s) => setWidth(s, w))}
            onReset={() => update((s) => setWidth(s, DEFAULT_WIDTH))}
            onStart={() => setResizing(true)}
            onEnd={() => setResizing(false)}
            className="hover:bg-sidebar-ring/40 focus-visible:bg-sidebar-ring/60"
          />
        )}
      </aside>

      {/* The area beside the sidebar is a 6 by 4 grid (frame/arrange.ts). The page and its dock sit together in one place of
          it, or, in a layout that gives each its share, in a place each. */}
      <div
        data-arrange-area
        className="relative grid min-w-0 flex-1"
        style={{ gridTemplateColumns: "repeat(6, minmax(0, 1fr))", gridTemplateRows: "repeat(4, minmax(0, 1fr))" }}
      >
        <div style={cell(arr.page)} className="flex min-h-0 min-w-0" data-arrange-page>
          <div className="relative flex min-w-0 flex-1 flex-col py-2 pr-2" style={{ order: dockFirst ? 1 : 0 }}>
            <div data-tauri-drag-region className="absolute inset-x-0 top-0 h-2" />
            <main
              data-page-area
              className="bg-background text-foreground flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border shadow-sm"
            >
              <header
                data-tauri-drag-region="deep"
                className={cn(
                  "@container flex h-11 shrink-0 items-center gap-2 border-b px-5 text-sm transition-[padding] duration-200",
                  hidden && trafficLights && "pl-[84px]",
                )}
              >
                {hidden && (
                  <button
                    type="button"
                    title="Show sidebar"
                    aria-label="Show sidebar"
                    onClick={() => update((s) => setMode(s, "expanded"))}
                    className="text-muted-foreground hover:bg-accent hover:text-foreground flex size-7 items-center justify-center rounded-md"
                  >
                    <PanelLeftOpen className="size-4" />
                  </button>
                )}
                {title}
                {actions && <div className="ml-auto flex items-center gap-1">{actions}</div>}
              </header>
              <div className="min-h-0 flex-1 overflow-auto">{children}</div>
            </main>
          </div>
          {arr.dock === null && dockNode ? (
            <div className="flex min-h-0" style={{ order: dockFirst ? 0 : 1 }}>
              {dockNode}
            </div>
          ) : null}
        </div>
        {arr.dock !== null && dockNode ? (
          <div style={cell(arr.dock)} className="flex min-h-0 min-w-0" data-arrange-dock>
            {dockNode}
          </div>
        ) : null}
        {onRestore && arranged(arr) ? (
          <button
            type="button"
            data-arrange-restore
            onClick={onRestore}
            title={
              layoutOff(arr)
                ? `Put the page back to full (${keysOf("layout-full")})`
                : "Put the boxes back in their first order"
            }
            className="bg-popover text-popover-foreground hover:bg-accent absolute right-4 bottom-4 z-10 flex items-center gap-2 rounded-full border px-3 py-1.5 text-[12px] shadow-md"
          >
            <span>{describe(arr) || "Boxes swapped"}</span>
            <span className="text-muted-foreground">{layoutOff(arr) ? `Full ${keysOf("layout-full")}` : "Reset"}</span>
          </button>
        ) : null}
      </div>
    </div>
  );
}
