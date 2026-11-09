import { type KeyboardEvent, useEffect, useRef, useState } from "react";
import { first, isAction, isHeading, isOff, last, type MenuEntry, reasonOf, step, typeAhead } from "./menu-core.ts";

// A plain dropdown over the shared menu model, enough for the Extensions menu. The Mac app's full context menu
// (`context-menu.tsx`: icons, submenus, search, placement) moves here with the frame in RFC 0217 C6 and replaces it.
export function MenuList({ label, entries, onClose }: { label: string; entries: MenuEntry[]; onClose: () => void }) {
  const [active, setActive] = useState(() => first(entries));
  const typed = useRef({ buffer: "", at: 0 });
  const list = useRef<HTMLDivElement>(null);

  useEffect(() => {
    list.current?.focus();
  }, []);

  const run = (i: number) => {
    const row = entries[i];
    if (!row || !isAction(row) || isOff(row)) return;
    onClose();
    row.run?.();
  };

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "ArrowDown") setActive((i) => step(entries, i, 1));
    else if (e.key === "ArrowUp") setActive((i) => step(entries, i, -1));
    else if (e.key === "Home") setActive(first(entries));
    else if (e.key === "End") setActive(last(entries));
    else if (e.key === "Enter" || e.key === " ") run(active);
    else if (e.key === "Escape" || e.key === "Tab") onClose();
    else if (e.key.length === 1) {
      const now = Date.now();
      const buffer = (now - typed.current.at < 700 ? typed.current.buffer : "") + e.key;
      typed.current = { buffer, at: now };
      const hit = typeAhead(entries, buffer, active);
      if (hit >= 0) setActive(hit);
    } else return;
    e.preventDefault();
  };

  return (
    <div
      ref={list}
      className="menu"
      role="menu"
      aria-label={label}
      tabIndex={-1}
      onKeyDown={onKey}
      aria-activedescendant={active >= 0 ? `menu-row-${active}` : undefined}
    >
      {entries.map((row, i) => {
        if (isHeading(row)) {
          return (
            <div key={`h-${row.heading}`} className="menu-heading" role="presentation">
              {row.heading}
            </div>
          );
        }
        if (!isAction(row)) return <hr key={`sep-before-${keyAfter(entries, i)}`} className="menu-sep" />;
        const off = isOff(row);
        return (
          <button
            type="button"
            key={row.id}
            id={`menu-row-${i}`}
            role="menuitem"
            aria-disabled={off || undefined}
            title={reasonOf(row) || undefined}
            tabIndex={-1}
            className={i === active ? "menu-row active" : "menu-row"}
            onMouseEnter={() => setActive(i)}
            onClick={() => run(i)}
          >
            <span>{row.label}</span>
            {row.hint ? <span className="menu-hint">{row.hint}</span> : null}
          </button>
        );
      })}
    </div>
  );
}

/** A separator's key: the id of the next action (tidy never leaves one at the end), so keys stay stable. */
function keyAfter(entries: MenuEntry[], i: number): string {
  const next = entries.slice(i + 1).find(isAction);
  return next ? next.id : "end";
}
