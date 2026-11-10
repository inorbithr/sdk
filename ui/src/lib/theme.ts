// The frame's tokens (frame/tokens.css) switch to dark under `html.dark`. The Mac app sets that class from its
// theme engine (src/lib/theme.ts: System, Light or Dark, and the chosen theme's values); this page has no theme
// choice yet, so it follows the system's setting and nothing else.

export function followSystemDark(root: HTMLElement = document.documentElement) {
  const query = window.matchMedia("(prefers-color-scheme: dark)");
  const apply = () => root.classList.toggle("dark", query.matches);
  apply();
  query.addEventListener("change", apply);
  return () => query.removeEventListener("change", apply);
}
