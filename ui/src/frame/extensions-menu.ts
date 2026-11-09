import type { Panel } from "../lib/extensions.ts";
import type { MenuEntry } from "./context-menu-core.ts";

/** The Extensions menu as the shared menu model: one row per panel, sorted as the server sent them. No heading: the
 *  menu is opened with the label "Extensions", which the context menu shows as its title and names it by. */
export function extensionsMenu(panels: Panel[], choose: (panel: Panel) => void): MenuEntry[] {
  if (panels.length === 0) {
    return [
      { id: "ext:none", label: "No extension panels installed", disabled: "Install one with `iohr ext install`" },
    ];
  }
  return panels.map((p) => ({
    id: `ext:${p.name}`,
    label: p.title,
    hint: `${p.name} ${p.version}`,
    run: () => choose(p),
  }));
}
