# Architecture

How the UI is put together, so you know where a change belongs. Back to the
[UI overview](../README.md).

## Two layers

| Layer | Path | Owns |
|---|---|---|
| Frame | `src/frame/` | The window: sidebar column, page inset with header and actions, a dock slot, the one context menu, resize handles, the layout grid, shortcuts, the panel sandbox |
| App | `src/*.tsx`, `src/lib/` | What goes in the slots: pages, menus' contents, state, the API client |

The frame owns no state. The app keeps each piece where it likes: this page keeps them in
`localStorage`, the Mac app keeps its sidebar in a file. The frame layer moved here from
the Mac app under ADR 0230 (see [AGENTS.md](../../AGENTS.md)); `pnpm run lib` compiles it to `lib/`
(declarations included) so the Mac app can import it. `package.json` exports only
`frame/tokens.css`, `frame/shortcuts.json` and `frame/*`.

## Pure core, thin shell

Most modules come as a pair: a `*-core.ts` that is plain data and functions (no React, no
DOM) and a store or component that wires it. Node tests the core directly.

| Core (tested) | Wired by |
|---|---|
| `frame/arrange.ts`, `arrange-menu.ts` | `lib/arrange-store.ts`, `app.tsx` |
| `frame/context-menu-core.ts` | `frame/context-menu.tsx` |
| `frame/rail-flyout-core.ts` | `frame/rail-flyout.tsx` |
| `frame/sidebar-frame.ts`, `dock-frame.ts` | `lib/sidebar-store.ts`, `lib/dock-store.ts` |
| `lib/home-core.ts` | `lib/home-data.ts`, `home.tsx` |
| `lib/settings-core.ts` | `lib/settings-data.ts`, `settings.tsx` |
| `lib/features-core.ts`, `orbits-core.ts`, `theme-core.ts` | their `*-store.ts`, `theme.ts` |

When you add logic, put the decision in the core and keep the component to drawing and
events.

## State kept in the browser

`lib/stored.ts` wraps one `localStorage` key with a parser that makes any stored value
whole, so a missing, old or hand-edited value falls back to the default. Storage can be
refused (a private window); the page then keeps the state for that load only. Nothing here
is sent to a server. Settings, Privacy lists what is held:

| Key | Holds |
|---|---|
| `iohr-ui.sidebar` | Hidden or shown, and its width |
| `iohr-ui.dock` | Open or closed, and its width |
| `iohr-ui.arrangement` | Window layout, and where the page and the dock sit |
| `iohr-ui.orbits` | The pages you pinned |
| `iohr-ui.features` | Pages put in maintenance, with reasons |
| `iohr-ui.theme` | System, Light or Dark |

Maintenance is a label on this machine: it shows a page as down in this browser and stops
nothing on a server.

## Routing

There is no router library. The server serves the one page for a path, and `app.tsx` reads
`window.location.pathname`:

- `/` is Home.
- `/settings` and `/settings/<pane>` are Settings.
- `/ext/<name>` is an extension's panel.

A panel needs its own route because the server fixes a page's `frame-src` when the page
loads. After a start, the page loads again on `/ext/<name>` and its policy allows the new
panel's loopback port. Close stops the panel and returns to `/`.

## Panels are sandboxed

An extension panel draws in an iframe whose attributes are fixed in one place,
`frame/sandbox.ts`: scripts may run, but there is no `allow-same-origin`, so the panel
gets an opaque origin and cannot read this page, its storage or `/api`. It cannot open
popups, submit forms, navigate the top window or ask for a device permission. The frame
address is plain HTTP on loopback and only a path under it (`panelSrc` in `lib/panels.ts`).

## Build constraints

`iohr ui serve` sends a strict content security policy: no inline script or style. The
Vite config therefore sets `assetsInlineLimit: 0` and builds no source maps. A change that
needs an inline script or style will not load in the served page.
