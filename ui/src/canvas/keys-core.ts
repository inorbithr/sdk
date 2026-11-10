// Which canvas key a key event is (PRD 0065). The rows are the page's keys file, shortcuts/pages/canvas.keys.json,
// the same file the registry of #524 lists; a bare letter never fires while a text field has the focus, a chord does.
import { type KeyLike, matches } from "../frame/shortcuts.ts";

export type KeyRow = { id: string; label: string; accel: string };

const bare = (accel: string) => !accel.includes("+") || accel.startsWith("Shift+");

/** The id of the row this event presses, or null. `inText`: a text field has the focus. */
export function pressed(rows: readonly KeyRow[], e: KeyLike, inText: boolean): string | null {
  for (const r of rows) {
    if (!matches(r.accel, e)) continue;
    if (inText && bare(r.accel) && r.accel !== "Escape") continue;
    return r.id;
  }
  return null;
}
