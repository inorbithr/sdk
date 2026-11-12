# Theming

How the page gets its colours, and how to change them. Back to the
[UI overview](../README.md).

## Tokens

`src/frame/tokens.css` is the one source of colour. It holds the console's design tokens as
CSS custom properties, light on `:root` and dark under `html.dark`, and maps them into
Tailwind with `@theme inline`. Components use the Tailwind names (`bg-card`,
`text-muted-foreground`, `border`, `bg-sidebar`) and never a literal colour.

| Group | Tokens |
|---|---|
| Surfaces | `--background`, `--card`, `--popover`, `--sidebar` and their `-foreground` |
| Actions | `--primary`, `--secondary`, `--accent`, `--destructive` |
| Lines | `--border`, `--input`, `--ring`, `--sidebar-border` |
| Status | `--success`, `--warning` |
| Brand | `--brand-amber` |
| Shape | `--radius` |
| Dark set | `--d-*`, for a surface that stays dark inside a light window |

The file is Apache-2.0 and shared with the Mac app (it moved from there under ADR 0230).
A token change is a change to both apps. Import it after `tailwindcss`, as
`src/styles.css` does.

## Light, dark and System

There is one theme, in two modes. The choice (System, Light or Dark) is kept in this
browser under `iohr-ui.theme` and set in Settings, Appearance. `startTheme()` in
`lib/theme.ts` toggles the `dark` class on `<html>`; System follows the operating system's
`prefers-color-scheme` and updates live. `lib/theme-core.ts` holds the pure part
(`parseChoice`, `isDark`) and has a test.

The Mac app also swaps whole themes. This page does not: it has the frame's one theme.

## The terminal themes

`iohr theme` and the terminal profile use a different, generated set; see
[the terminal theme](../../cli/docs/terminal-theme.md). They do not share files with this
page.

## Change a colour

1. Edit the token in `src/frame/tokens.css`, in both the `:root` and the `html.dark`
   block.
2. Check contrast in light and dark, in the sidebar and in a menu.
3. Run `mise run ui:check`.
