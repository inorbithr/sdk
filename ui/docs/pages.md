# Pages

One short section for each page that is on `main` today. Back to the
[UI overview](../README.md). The sidebar lists Home, Settings and each installed
extension that declares a panel.

## Home

The welcome page, at `/`. Io tells the day as a few plain sentences, each marked back to
the event that proves it, newest first on a time rail. A filter narrows the rail to All,
Problems, Decisions or Projects. A first run, with nothing yet, says so.

- Reads: your profile, reviews waiting, spaces, monitors, incidents and each monitor's
  recent checks ([what Home reads](api-and-proxy.md#what-home-reads)), refreshed every
  30 seconds.
- Monitors are shown as services. A paused monitor is not watched, and a health the API
  did not give shows as "not known", never "up".
- Signed out, it offers Sign in (`iohr login --web`; your browser opens).
- Source: `src/home.tsx`, `src/lib/home-core.ts`, `src/lib/home-data.ts`.

## Settings

At `/settings`, with the chosen pane at `/settings/<pane>`. A search box over the pane list
keeps the panes whose name, description or location match. The panes keep the Mac app's
names and order.

| Pane | On this page |
|---|---|
| General | Sidebar and window layout |
| Appearance | System, Light or Dark ([theming](theming.md)) |
| Accounts | Who you are signed in as; Sign in and Sign out |
| Team | The members of the account you work in |
| Projects | The account's spaces, with document and review counts |
| Keyboard | Every shortcut ([keyboard shortcuts](keyboard-shortcuts.md)) |
| Features & maintenance | Put a page in maintenance, with a reason and a "back at" |
| Privacy & data | What this page keeps in this browser |
| Developer | The reads the pages make, sign-in state and the extension listing |
| About | Address, installed panel count and how to start the page |

Setup points to `iohr init` in a terminal. The panes AI providers, Server, Worker, Builds,
Repositories, Code, Terminal & remote hosts, Notifications, Coherence, Roles and Operations
open a page that says they live in the Mac app today; nothing is faked here.

Source: `src/settings.tsx`, `src/settings-panes.tsx`, `src/lib/settings-core.ts`.

## Extension panels

At `/ext/<name>`. An installed extension that declares a `[panel]` in its manifest gets a
sidebar row. Choosing it starts the panel (the server runs it with the full trust check,
the same one `iohr ext` uses), then loads the page on its own route and draws the panel in a
sandboxed frame ([architecture](architecture.md#panels-are-sandboxed)). Close stops the
panel and returns Home. See [extensions](../../cli/docs/extensions.md) for how a panel is
declared and installed.

Source: `src/frame/panel-frame.tsx`, `src/lib/extensions.ts`, `src/lib/panels.ts`.

## Around the pages

These belong to the frame, not to one page:

- **Sidebar**: hide it with `Cmd+\`; drag its edge to resize.
- **Details dock**: the Details button opens a box beside the page that says what the
  listing knows about the page in front.
- **Extensions menu** in the page header, and a right click on an extension row, share the
  one context menu.
- **Orbits** pin pages and step between them, like Spaces on macOS.
- **Maintenance** marks a page as under maintenance on this machine (a label, not a switch
  on any server), with Undo for ten seconds.
- **Layout** places the page and the dock on a 6 by 4 grid in a window 900 px wide or more.

## In progress

RFC 0217 continues in open pull requests: atlas, teams, planner, sensors, canvas, 3D,
shortcuts, decisions and live. None of them is described above as shipped. Each pull
request adds its own section here, in the same pull request, in the shape below, and
removes its name from this list.

````md
## <Page name>

One sentence: what the person gets. Route: `/<route>`.

- Reads: the API paths it calls (through the proxy, GET only).
- Writes: none, or the server verb it calls.
- Empty, signed-out and error states, in one line each.
- Stub parts, labelled as stubs.
- Source: the files under `src/`.
````
