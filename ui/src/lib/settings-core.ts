// Settings, the pure part (RFC 0217, slice 4). The Mac app's Settings is a list of panes like macOS System Settings
// (iohr-mac src/lib/settings-groups.ts: 22 of them). This page has the same list under the same names, in the same
// order, and every entry opens a page. A pane this page can do is `built`; one that only the Mac app or a terminal
// does says so and where, and never shows a control that does nothing. No React, no DOM, so node tests it.

export type PaneId =
  | "setup"
  | "general"
  | "appearance"
  | "accounts"
  | "team"
  | "ai"
  | "server"
  | "worker"
  | "builds"
  | "repositories"
  | "projects"
  | "code"
  | "terminal"
  | "keyboard"
  | "notifications"
  | "features"
  | "coherence"
  | "personalities"
  | "privacy"
  | "operations"
  | "developer"
  | "about";

export type Pane = {
  id: PaneId;
  label: string;
  /** What the pane is for, as the Mac app says it. */
  blurb: string;
  /** True when this page does it; false when it lives somewhere else, named in `where`. */
  built: boolean;
  /** For a pane that is not built here: where it does work today. */
  where: string;
};

const MAC = "the Mac app";
const pane = (id: PaneId, label: string, blurb: string, where = ""): Pane => ({
  id,
  label,
  blurb,
  built: where === "",
  where,
});

/** The Mac app's panes, in its order. */
export const PANES: Pane[] = [
  pane("setup", "Setup", "Run any step of first-run setup again.", "a terminal: `iohr init`"),
  pane("general", "General", "How InOrbit behaves here."),
  pane("appearance", "Appearance", "Light and dark."),
  pane("accounts", "Accounts", "Who you are signed in as."),
  pane("team", "Team", "The people of the account you work in."),
  pane("ai", "AI providers", "Claude accounts, the model rule, limits and hooks.", MAC),
  pane("server", "Server", "The build host the app watches over ssh, and how it is doing.", MAC),
  pane("worker", "Worker", "The background helper that keeps watch while the app is closed.", MAC),
  pane("builds", "Builds", "Beta builds of InOrbit for Mac: the queue, the log and the betas kept.", MAC),
  pane("repositories", "Repositories", "Which repositories and folders the app works with.", MAC),
  pane("projects", "Projects", "Your projects."),
  pane("code", "Code", "Which repositories the editor may open, and what it can reach.", MAC),
  pane("terminal", "Terminal & Remote hosts", "Where terminals open and which hosts they reach.", MAC),
  pane("keyboard", "Keyboard", "Shortcuts."),
  pane("notifications", "Notifications", "What tells you, and when it stays quiet.", MAC),
  pane("features", "Features & maintenance", "Put pages in maintenance."),
  pane("coherence", "Coherence", "Checks that the app and the platform agree.", MAC),
  pane("personalities", "Roles", "How agents sound and behave.", MAC),
  pane("privacy", "Privacy & data", "What this page keeps in this browser."),
  pane("operations", "Operations", "Cloud and this machine: what lives where, and what may leave.", MAC),
  pane("developer", "Developer", "Technical details of this page."),
  pane("about", "About", "What is serving this page."),
];

export const SETTINGS = "/settings";
export const DEFAULT_PANE: PaneId = "general";

export const paneById = (id: string): Pane | undefined => PANES.find((p) => p.id === id);
export const paneHref = (id: PaneId): string => `${SETTINGS}/${id}`;

/** Where the page's path points in Settings: not Settings (null), a pane, or a name that is no pane. */
export type SettingsRoute = { pane: Pane } | { unknown: string };

export function settingsRoute(pathname: string): SettingsRoute | null {
  const m = pathname.match(/^\/settings(?:\/([^/]*))?\/?$/);
  if (!m) return null;
  const name = m[1] ?? "";
  if (name === "") return { pane: paneById(DEFAULT_PANE) as Pane };
  const found = paneById(name);
  return found ? { pane: found } : { unknown: name };
}

/** The panes a search keeps: label, blurb and where it lives, any case; empty keeps all, in order. */
export function matchPanes(query: string): Pane[] {
  const q = query.trim().toLowerCase();
  return q === "" ? PANES : PANES.filter((p) => `${p.label} ${p.blurb} ${p.where}`.toLowerCase().includes(q));
}

/** What this page keeps in this browser, by key: the one list Privacy shows and clears. */
export const HELD: { key: string; label: string }[] = [
  { key: "iohr-ui.sidebar", label: "Sidebar: hidden or shown, and its width" },
  { key: "iohr-ui.dock", label: "Details dock: open or closed, and its width" },
  { key: "iohr-ui.arrangement", label: "Window layout and where the page and the dock sit" },
  { key: "iohr-ui.orbits", label: "Orbits: the pages you pinned" },
  { key: "iohr-ui.features", label: "Pages put in maintenance, with their reasons" },
  { key: "iohr-ui.theme", label: "Light, dark or System" },
];

/** The keys of `HELD` that have something stored now. */
export const heldNow = (has: (key: string) => boolean): string[] => HELD.map((h) => h.key).filter(has);
