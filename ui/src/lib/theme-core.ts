// The page's colour choice, the pure part (RFC 0217, slice 4). The Mac app lets a person choose System, Light or Dark
// (iohr-mac src/lib/theme.ts also swaps whole themes; this page has the frame's one theme, light and dark). No React,
// no DOM, so node tests it.

export type Choice = "system" | "light" | "dark";

export const CHOICES: { id: Choice; label: string }[] = [
  { id: "system", label: "System" },
  { id: "light", label: "Light" },
  { id: "dark", label: "Dark" },
];

/** A stored choice made whole: anything else is System, the page's behaviour before there was a choice. */
export const parseChoice = (o: unknown): Choice => (o === "light" || o === "dark" ? o : "system");

/** Is the page dark, given the choice and what the system says? */
export const isDark = (choice: Choice, systemDark: boolean): boolean =>
  choice === "system" ? systemDark : choice === "dark";
