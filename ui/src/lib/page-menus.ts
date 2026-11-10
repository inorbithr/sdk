// The Orbits menu and the Maintenance menu as menu rows (frame/context-menu-core.ts), after the Mac app's header
// buttons (iohr-mac src/components/orbits-button.tsx and maintenance-button.tsx). Pure: the caller says what each row
// does, so node tests the rows and when they are off.

import { group, type MenuEntry } from "../frame/context-menu-core.ts";
import { entryOf, FEATURES, isDown, type States, whyClosed } from "./features-core.ts";
import { HOME, has, MAX_ORBITS, neighbour, type Orbits } from "./orbits-core.ts";

export type PageInfo = { key: string; title: string };

export type OrbitActions = {
  go: (page: string) => void;
  add: (page: string) => void;
  remove: (page: string) => void;
};

/**
 * The orbits: each picked page with a check on the one you are on, then adding or removing this page, then the step to
 * the previous or next orbit. `pages` is what exists now; a picked page that no longer exists is not listed.
 */
export function orbitsMenu(o: Orbits, current: string, pages: PageInfo[], act: OrbitActions): MenuEntry[] {
  const title = (k: string) => pages.find((p) => p.key === k)?.title;
  const listed = o.pages.filter((k) => title(k) !== undefined);
  const prev = neighbour(o, current, -1);
  const next = neighbour(o, current, 1);
  const here = title(current) ?? current;
  const addOff = has(o, current)
    ? "Already an orbit"
    : o.pages.length >= MAX_ORBITS
      ? `At most ${MAX_ORBITS} orbits`
      : false;
  const removeOff = current === HOME ? "Home is always an orbit" : has(o, current) ? false : "Not an orbit";
  return group(
    listed.map((k) => ({
      id: `orbit:${k}`,
      label: title(k) as string,
      icon: "orbit",
      checked: k === current,
      run: () => act.go(k),
    })),
    [
      {
        id: "orbit:add",
        label: `Add ${here} to the orbits`,
        icon: "orbit",
        disabled: addOff,
        run: () => act.add(current),
      },
      {
        id: "orbit:remove",
        label: `Remove ${here} from the orbits`,
        icon: "stop",
        disabled: removeOff,
        run: () => act.remove(current),
      },
    ],
    [
      {
        id: "orbit:prev",
        label: "Previous orbit",
        icon: "left",
        disabled: prev === null ? "No orbit before this one" : false,
        run: () => prev !== null && act.go(prev),
      },
      {
        id: "orbit:next",
        label: "Next orbit",
        icon: "right",
        disabled: next === null ? "No orbit after this one" : false,
        run: () => next !== null && act.go(next),
      },
    ],
  );
}

export type MaintenanceActions = {
  /** Opens the form for a feature: its reason and when it is back. */
  edit: (id: string) => void;
  back: (id: string) => void;
  allOnline: () => void;
  undo: (() => void) | null;
  undoLabel: string;
};

/** Each page with its state (a down one says why), Bring everything online, and Undo while the last change can go back. */
export function maintenanceMenu(s: States, act: MaintenanceActions): MenuEntry[] {
  const down = FEATURES.filter((f) => isDown(entryOf(s, f.id)));
  return group(
    FEATURES.map((f) => {
      const e = entryOf(s, f.id);
      return {
        id: `maintenance:${f.id}`,
        label: f.name,
        icon: "wrench",
        checked: isDown(e),
        hint: isDown(e) ? whyClosed(e) : "online",
        run: () => (isDown(e) ? act.back(f.id) : act.edit(f.id)),
      };
    }),
    [
      {
        id: "maintenance:all",
        label: "Bring everything online",
        icon: "reset",
        disabled: down.length === 0 ? "Nothing is in maintenance" : false,
        run: act.allOnline,
      },
      ...(act.undo ? [{ id: "maintenance:undo", label: `Undo: ${act.undoLabel}`, icon: "undo", run: act.undo }] : []),
    ],
  );
}
