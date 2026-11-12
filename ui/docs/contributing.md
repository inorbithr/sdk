# Contributing and testing

How to change the UI and what a pull request needs. The repository-wide rules are in
[CONTRIBUTING.md](../../CONTRIBUTING.md); this page adds what is specific to `ui/`. Back to
the [UI overview](../README.md).

## Set up

```sh
mise install          # pinned Node, pnpm and the rest
mise run ui:install   # pnpm install --frozen-lockfile in ui/
mise run ui:check     # must pass before you push
```

## Rules for this package

- Components draw; decisions live in a `*-core.ts` that Node can test
  ([architecture](architecture.md#pure-core-thin-shell)).
- No literal colours; use the tokens ([theming](theming.md)).
- No inline script or style: the served page's policy refuses both.
- The page holds no credential and writes nothing through the proxy
  ([API and proxy](api-and-proxy.md)).
- A value the API did not give is shown as not known, never filled in. A placeholder is
  labelled a stub on the page.
- Dependencies stay pinned in `package.json` and the lockfile; a bump changes both.
- User-visible text follows the [writing style](../../docs/style.md): buttons are verbs,
  errors say what happened and what to do.

## Tests

`pnpm run test` runs `node --test` over `test/*.test.ts` with no browser and no React. A
test imports a core module and asserts on plain data. The API client takes a `fetch`
argument, so `test/api.test.ts` passes a fake and checks the request it saw, including that
no credential is sent and that a `401` reads as signed out.

| File | Covers |
|---|---|
| `api.test.ts`, `panels.test.ts`, `extensions.test.ts` | The client and the panel seam |
| `arrange*.test.ts`, `dock.test.ts`, `sidebar-frame.test.ts`, `rail-flyout.test.ts` | Layout and frame geometry |
| `context-menu.test.ts`, `page-menus.test.ts` | Menu models |
| `home-core.test.ts`, `settings-*.test.ts`, `features-core.test.ts`, `orbits-core.test.ts`, `theme-core.test.ts` | The pages' pure logic |
| `shortcuts.test.ts` | Accelerator spelling and matching |

A bug fix starts with a test that fails on `main` and passes on your branch.

## Add a page

1. Put the decisions in `src/lib/<page>-core.ts` with a test in `test/`.
2. Put the reads in `src/lib/<page>-data.ts` through `apiGet`; each read stands alone.
3. Draw it in `src/<page>.tsx` and route it in `app.tsx`; the server must serve the path.
4. List it in `FEATURES` in `lib/features-core.ts` if it can be put in maintenance.
5. Write its section in the [pages guide](pages.md) in the same pull request.

## Pull request

- Title: `feat(cli): ...` for UI changes (the title check has no `ui` scope; the UI ships
  through `iohr ui`), `docs(docs): ...` for documentation only.
- Describe what changed and how you checked it. A UI change shows the page, in light and
  dark.
- The CI `ui` job runs when `ui/**` changes. `ci-ok` is the required check.
