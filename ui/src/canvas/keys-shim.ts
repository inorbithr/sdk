// A thin stand-in for `useShortcuts(scope, handlers)` of the shortcut registry (draft PR #524, lib/use-shortcuts.ts):
// the same call, the same ids, read from the same keys file (shortcuts/pages/canvas.keys.json). When #524 is on main,
// canvas-page.tsx imports useShortcuts from "../lib/use-shortcuts.ts" instead and this file and keys-core.ts go.
import { useEffect, useRef } from "react";
import table from "../shortcuts/pages/canvas.keys.json" with { type: "json" };
import { type KeyRow, pressed } from "./keys-core.ts";

const inTextField = (t: EventTarget | null) =>
  t instanceof HTMLElement && (t.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName));

export function useShortcuts(scope: "canvas", handlers: Partial<Record<string, () => void>>) {
  const now = useRef(handlers);
  now.current = handlers;
  useEffect(() => {
    const rows = (scope === "canvas" ? table.rows : []) as KeyRow[];
    const on = (e: KeyboardEvent) => {
      const id = pressed(rows, e, inTextField(e.target));
      const run = id ? now.current[id] : undefined;
      if (!run) return;
      e.preventDefault();
      run();
    };
    window.addEventListener("keydown", on, true);
    return () => window.removeEventListener("keydown", on, true);
  }, [scope]);
}
