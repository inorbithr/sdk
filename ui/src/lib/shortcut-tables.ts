// Every table of the registry, loaded (the pages' files by the bundler's glob). Not pure: node tests read the same
// files from disk (test/shortcut-registry.test.ts).
import layoutTable from "../frame/layout-shortcuts.json" with { type: "json" };
import frameTable from "../frame/shortcuts.json" with { type: "json" };
import reservedTable from "../shortcuts/browser-reserved.json" with { type: "json" };
import terminalTable from "../shortcuts/terminal-keys.json" with { type: "json" };
import webTable from "../shortcuts/web-shortcuts.json" with { type: "json" };
import { type Binding, flatten, type Group, type Reserved, type Row } from "./shortcut-registry.ts";

const pageFiles = import.meta.glob<Omit<Group, "source">>("../shortcuts/pages/*.keys.json", {
  eager: true,
  import: "default",
});

export const GROUPS: Group[] = [
  { scope: "global", title: "Frame", source: "frame", rows: frameTable as Row[] },
  { scope: "global", title: "Window layout", source: "layout", where: "Where the window is wide enough to arrange", rows: layoutTable as Row[] },
  { scope: "global", title: "Go to", source: "web", rows: webTable as Row[] },
  ...Object.values(pageFiles).map((g) => ({ ...g, source: "page" as const })),
];
export const BINDINGS: Binding[] = flatten(GROUPS);
export const RESERVED: Reserved[] = reservedTable.keys as Reserved[];
export const TERMINAL = terminalTable.rows;
