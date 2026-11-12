// The frame's tokens (frame/tokens.css) switch to dark under `html.dark`. The Mac app sets that class from its theme
// engine (src/lib/theme.ts: System, Light or Dark, and the chosen theme's values); this page has System, Light and
// Dark (Settings, Appearance), kept in this browser. System follows the system's setting live.
import { stored } from "./stored.ts";
import { type Choice, isDark, parseChoice } from "./theme-core.ts";

const choice = stored<Choice>("iohr-ui.theme", parseChoice);

export const useThemeChoice = choice.use;
export const setThemeChoice = (c: Choice) => choice.update(() => c);

/** Puts the chosen theme on the page now and keeps it right when the choice or the system's setting changes. */
export function startTheme(root: HTMLElement = document.documentElement) {
  const query = window.matchMedia("(prefers-color-scheme: dark)");
  let current = parseChoice(choice.get());
  const apply = () => root.classList.toggle("dark", isDark(current, query.matches));
  apply();
  query.addEventListener("change", apply);
  const off = choice.subscribe(() => {
    current = parseChoice(choice.get());
    apply();
  });
  return () => {
    query.removeEventListener("change", apply);
    off();
  };
}
