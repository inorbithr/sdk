# InOrbit web UI

The page `iohr ui serve` serves on your own machine: a welcome page, a frame with a
sidebar and a dock, Settings, and the panels of the extensions you installed. It is a
private Vite and React package, built once and served as static files. It is not
published to a registry. Back to the [repository overview](../README.md).

Status: pre-1.0, like `iohr`. The design is [RFC 0217](https://docs.inorbit.hr), and the
shell frame layer moved here from the Mac app under ADR 0230 (see the `ui/` row in [AGENTS.md](../AGENTS.md)). Pages that
are still open pull requests are not described here as shipped; the
[pages guide](docs/pages.md#in-progress) says where each one will be documented.

## Run it

```sh
iohr ui              # serve the built page on loopback and open your browser
iohr ui serve --open # the same, spelled out
iohr ui open         # open the page a running serve recorded
```

`iohr ui serve` needs a built `dist/`. From a checkout of this repository:

```sh
mise run ui:build    # pnpm install --frozen-lockfile, then `iohr ui build`
mise run ui:serve    # serve it
```

The page opens through a one-time launch link. Every flag, the sign-in model and what the
server refuses is in [`iohr ui`](../cli/docs/commands.md#iohr-ui); this folder does not
repeat it.

## Develop

```sh
mise run ui:dev      # the Vite dev server (`iohr ui dev`), on loopback only
```

To point the dev server's `/api` at a running `iohr ui serve`, set `IOHR_UI_API` to that
server's loopback address before starting it. The dev server does not insert the
per-serve token the built page carries, so a page that reads the API shows "Not
connected" there. Work on the frame, menus and layout in dev; check anything that calls
the API against the built page from `iohr ui serve`.

## Check

```sh
mise run ui:check    # install, then `iohr ui check` (pnpm run check)
```

`pnpm run check` runs, in order: `biome ci .`, the type check, the tests, the build and the
`lib` build. Each also runs alone:

| Script | What it does |
|---|---|
| `pnpm run dev` | Vite dev server |
| `pnpm run build` | Type check, then `vite build` into `dist/` |
| `pnpm run lib` | Compile `src/frame` to `lib/` (what the Mac app imports; never committed) |
| `pnpm run typecheck` | `tsc --noEmit` |
| `pnpm run test` | `node --test` over `test/*.test.ts` |
| `pnpm run check` | All of the above, plus Biome |

Node 22.18 or later and the pinned pnpm are required; `mise install` at the repository
root installs both.

## Layout

```text
ui/
  index.html          the one page; Vite's entry
  src/
    main.tsx          mounts <App/> and starts the theme
    app.tsx           the shell: sidebar, header menus, dock, page routing
    home.tsx          Home, the welcome page
    settings.tsx      Settings: the pane list and search
    settings-panes.tsx  one component per pane
    session-control.tsx sign-in and sign-out in the header and sidebar
    maintenance-form.tsx put a page in maintenance
    styles.css        Tailwind entry and the page's base styles
    frame/            the shell frame layer shared with the Mac app
    lib/              state, API client, pure logic
  test/               node:test files, one per pure module
  docs/               the guides below
```

## Guides

| Guide | What it covers |
|---|---|
| [Architecture](docs/architecture.md) | Frame and app, pure core and React shell, state kept in the browser, the `lib` build |
| [Pages](docs/pages.md) | Home, Settings, extension panels, and where pages still in progress will be written up |
| [API and proxy](docs/api-and-proxy.md) | How the page reads the API without holding a credential |
| [Keyboard shortcuts](docs/keyboard-shortcuts.md) | Every shortcut, from the two JSON tables |
| [Theming](docs/theming.md) | Tokens, light and dark, the one theme |
| [Contributing and testing](docs/contributing.md) | Adding a page, writing a test, the pull request |

Related: [`iohr ui` reference](../cli/docs/commands.md#iohr-ui), [extensions and
panels](../cli/docs/extensions.md), [repository contributing guide](../CONTRIBUTING.md).
