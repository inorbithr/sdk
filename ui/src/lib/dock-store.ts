import { parseDock } from "../frame/dock-frame.ts";
import { stored } from "./stored.ts";

// This page's dock state: whether the right dock is open and its width (lib/stored.ts). iohr-mac keeps a width per
// panel in its side-panels store; this page has one dock.

const dock = stored("iohr-ui.dock", parseDock);

export const updateDock = dock.update;

export function useDock() {
  return [dock.use(), updateDock] as const;
}
