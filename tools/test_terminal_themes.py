#!/usr/bin/env python3
"""Unit tests of tools/terminal-themes.py: what it reads from the app's term-config.ts, and
that the committed themes.json is whole."""

import importlib.util
import json
import pathlib
import unittest

HERE = pathlib.Path(__file__).parent
spec = importlib.util.spec_from_file_location("themes", HERE / "terminal-themes.py")
themes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(themes)


def colours(shift=0):
    return ",\n".join(f'    {k}: "#{(i * 7 + shift) % 256:02x}{(i * 3) % 256:02x}{(i * 5) % 256:02x}"' for i, k in enumerate(themes.COLOURS))


TS = f"""
const INORBIT: ITheme = {{
{colours(0)},
}};
export const THEMES: Record<string, {{ label: string; dark: boolean; theme: ITheme }}> = {{
  inorbit: {{ label: "InOrbit", dark: true, theme: INORBIT }},
  "solarized-dark": {{
    label: "Solarized dark",
    dark: true,
    theme: {{
{colours(40)},
    }},
  }},
  light: {{ label: "Light", dark: false, theme: {{
{colours(90)},
  }} }},
}};
export const DEFAULTS: TermConfig = {{
  fontFamily: "Geist Mono",
  fontSize: 12.5,
  cursor: "bar",
  cursorBlink: true,
  theme: "inorbit",
}};
export const fontStack = (c: TermConfig) => `"${{c.fontFamily}}", "SF Mono", Menlo, monospace`;
"""


class Parse(unittest.TestCase):
    def test_reads_every_theme_in_order_with_every_colour(self):
        cat = themes.parse(TS)
        self.assertEqual([t["id"] for t in cat["themes"]], ["inorbit", "solarized-dark", "light"])
        self.assertEqual([t["dark"] for t in cat["themes"]], [True, True, False])
        for t in cat["themes"]:
            self.assertEqual(sorted(t["colors"]), sorted(themes.COLOURS))
        self.assertEqual(themes.whole(cat), [])

    def test_a_theme_that_reuses_the_inorbit_constant_gets_its_colours(self):
        cat = themes.parse(TS)
        inorbit = cat["themes"][0]["colors"]
        self.assertEqual(inorbit["background"], "#000000")
        self.assertNotEqual(inorbit, cat["themes"][1]["colors"])

    def test_the_font_and_the_cursor_come_from_the_defaults(self):
        cat = themes.parse(TS)
        self.assertEqual(cat["font"], {"family": "Geist Mono", "fallbacks": ["SF Mono", "Menlo"], "size": 12.5})
        self.assertEqual(cat["cursor"], {"style": "bar", "blink": True})
        self.assertEqual(cat["default"], "inorbit")

    def test_a_missing_colour_is_named(self):
        broken = TS.replace('brightCyan: "', 'brightCyanX: "', 1)
        with self.assertRaises(SystemExit):
            themes.parse(broken)

    def test_render_is_stable(self):
        cat = themes.parse(TS)
        self.assertEqual(themes.render(cat), themes.render(json.loads(themes.render(cat))))


class Committed(unittest.TestCase):
    def test_the_committed_catalogue_is_whole(self):
        cat = json.loads(themes.OUT.read_text())
        self.assertEqual(themes.whole(cat), [])
        self.assertEqual([t["id"] for t in cat["themes"]], ["inorbit", "solarized-dark", "nord", "light"])


if __name__ == "__main__":
    unittest.main()
