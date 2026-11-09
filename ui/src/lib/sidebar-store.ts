import type { UpdateSidebar } from "../frame/frame.tsx";
import { parseFrame } from "../frame/sidebar-frame.ts";
import { stored } from "./stored.ts";

// This page's sidebar state: the frame's mode and width (lib/stored.ts). It follows iohr-mac's
// src/lib/sidebar-store.ts, which keeps more (pins, projects) in a file.

const sidebar = stored("iohr-ui.sidebar", parseFrame);

export const updateSidebar: UpdateSidebar = sidebar.update;

export function useSidebar() {
  return [sidebar.use(), updateSidebar] as const;
}
