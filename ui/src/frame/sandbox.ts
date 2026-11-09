// The frame an extension panel draws in. Its attributes are fixed here and nowhere else: scripts may run, but the
// panel gets an opaque origin (no `allow-same-origin`), so it cannot read this page, its storage or `/api`; it
// cannot open popups, submit forms, navigate the top window or ask for any device permission.
export const PANEL_FRAME = {
  sandbox: "allow-scripts",
  referrerPolicy: "no-referrer",
  allow: "",
} as const;
