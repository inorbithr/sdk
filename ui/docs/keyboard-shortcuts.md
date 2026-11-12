# Keyboard shortcuts

The shortcuts the frame answers, and where they are defined. Back to the
[UI overview](../README.md).

## The table

`Cmd` is `Ctrl` off the Mac. Settings, Keyboard shows the same list.

| Shortcut | Action | Id |
|---|---|---|
| `Cmd+\` | Toggle sidebar | `sidebar` |
| `Ctrl+Alt+Left` | Left half | `layout-left` |
| `Ctrl+Alt+Right` | Right half | `layout-right` |
| `Ctrl+Alt+Up` | Top half | `layout-top` |
| `Ctrl+Alt+Down` | Bottom half | `layout-bottom` |
| `Ctrl+Alt+Enter` | Full | `layout-full` |
| `Ctrl+Alt+G` | Grid picker | `layout-grid` |
| `Ctrl+Alt+Z` | Back to before | `layout-back` |

The layout shortcuts arrange the page and its dock inside the window on a 6 by 4 grid.
They work only while the window is at least 900 px wide (`ARRANGE_MIN_WIDTH` in
`frame/frame.tsx`); below that they do nothing. A browser tab cannot move its
own window, so "left half" means the left half of the page area.

## Where they are defined

| File | Holds | Read by |
|---|---|---|
| `src/frame/shortcuts.json` | The frame's own shortcuts: `id`, `label`, `accel` | This page and the Mac app (its page and its native menu) |
| `src/frame/layout-shortcuts.json` | The layout shortcuts, same shape | This page |

The layout rows are a separate file because the Mac app already keeps its own table of
them and reads `shortcuts.json` for its native menu. They fold into one file when ADR 0230
is extended to the layouts.

## Spelling and matching

An accelerator is spelt `Modifier+Key`: `CmdOrCtrl`, `Cmd`, `Ctrl`, `Alt`, `Shift`, then a
key name (`ArrowLeft`, `Enter`, or one letter). `src/frame/shortcuts.ts` provides:

- `canon` gives one spelling per accelerator.
- `symbols` shows it with Mac glyphs (`Ctrl+Alt+G` as `⌃⌥G`).
- `matches` tests a key event. A letter also matches by its physical key, because Alt turns
  letters into other characters.
- `useFrameShortcuts(handlers)` listens on the window, in the capture phase, and runs the
  handler of the shortcut pressed. An id with no handler passes through to the browser.

## Add a shortcut

1. Add a row to the right JSON file. Keep the id unique.
2. Pass a handler for that id to `useFrameShortcuts` in `app.tsx`.
3. Add a case to `test/shortcuts.test.ts`.

A new row in `shortcuts.json` also reaches the Mac app's native menu, so say so in the
pull request.
