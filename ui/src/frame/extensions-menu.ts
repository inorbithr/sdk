import type { Panel } from "../lib/extensions.ts";
import { heading, type MenuEntry, tidy } from "./context-menu-core.ts";

/** The Extensions menu as the shared menu model: a heading, then one row per panel, sorted as the server sent them. */
export function extensionsMenu(panels: Panel[], choose: (panel: Panel) => void): MenuEntry[] {
  if (panels.length === 0) {
    return [
      heading("Extensions"),
      { id: "ext:none", label: "No extension panels installed", disabled: "Install one with `iohr ext install`" },
    ];
  }
  return tidy([
    heading("Extensions"),
    ...panels.map((p) => ({
      id: `ext:${p.name}`,
      label: p.title,
      hint: `${p.name} ${p.version}`,
      run: () => choose(p),
    })),
  ]);
}
