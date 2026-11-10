import { useEffect, useMemo, useRef, useState } from "react";
import { cn } from "./frame/cn.ts";
import { isMac, show } from "./frame/shortcuts.ts";
import type { Binding } from "./lib/shortcut-registry.ts";
import { BINDINGS, TERMINAL } from "./lib/shortcut-tables.ts";

// The one place the registry is drawn: the help overlay (? or ⌘/), the palette (⌘K) and the Settings Keyboard pane
// all read BINDINGS, so a key added to a table shows up here without a second list.

const mac = () => isMac(typeof navigator === "undefined" ? "" : navigator.platform);

/** A binding's key as the person reads it: glyphs on a Mac, words elsewhere; a sequence reads "g then d". */
export function keysShown(b: Binding, onMac = mac()): string {
  if (b.seq) return b.seq.join(" then ");
  return show(b.accel ?? "", onMac);
}

const kbd = "bg-muted rounded px-1.5 py-0.5 font-mono text-[12px]";

/** What a page's go row needs to be live: its page exists in this build. */
const live = (b: Binding, pages: string[]) => b.page === undefined || pages.includes(b.page);

export function ShortcutList({ pages }: { pages: string[] }) {
  const groups = useMemo(() => {
    const m = new Map<string, Binding[]>();
    for (const b of BINDINGS) m.set(b.title, [...(m.get(b.title) ?? []), b]);
    return [...m];
  }, []);
  return (
    <div className="flex flex-col gap-4 text-[13px]">
      {groups.map(([title, rows]) => (
        <section key={title}>
          <h3 className="mb-1 text-sm font-semibold">{title}</h3>
          {rows[0].where ? <p className="text-muted-foreground mb-1">{rows[0].where}</p> : null}
          {rows.map((b) => (
            <div key={b.id} className="flex min-h-8 items-center justify-between gap-3 border-t py-1 first:border-t-0">
              <span className="min-w-0">
                {b.label}
                {live(b, pages) ? null : <span className="text-muted-foreground"> (not in this build yet)</span>}
              </span>
              <kbd className={kbd}>{keysShown(b)}</kbd>
            </div>
          ))}
        </section>
      ))}
      <section>
        <h3 className="mb-1 text-sm font-semibold">In the terminal panel (iohr)</h3>
        <p className="text-muted-foreground mb-1">Where a key differs from the web, the note says so.</p>
        {TERMINAL.map((t) => (
          <div key={t.id} className="flex min-h-8 items-center justify-between gap-3 border-t py-1 first:border-t-0">
            <span className="min-w-0">
              {t.label}
              {t.agree === "differs" ? (
                <span className="text-muted-foreground"> (web: {t.note ?? "different key"})</span>
              ) : null}
            </span>
            <kbd className={kbd}>{t.keys}</kbd>
          </div>
        ))}
      </section>
    </div>
  );
}

function Modal({ label, onClose, children }: { label: string; onClose: () => void; children: React.ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    ref.current?.focus();
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("keydown", esc);
      before?.focus?.();
    };
  }, [onClose]);
  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center p-4 pt-[12vh]">
      <button
        type="button"
        tabIndex={-1}
        aria-label="Close"
        onClick={onClose}
        className="absolute inset-0 cursor-default bg-black/40"
      />
      <div
        ref={ref}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        className="bg-card text-card-foreground relative max-h-[75vh] w-full max-w-lg overflow-auto rounded-2xl border p-4 shadow-lg outline-none"
      >
        {children}
      </div>
    </div>
  );
}

export function HelpOverlay({ pages, onClose }: { pages: string[]; onClose: () => void }) {
  return (
    <Modal label="Keyboard shortcuts" onClose={onClose}>
      <div className="mb-3 flex items-center justify-between">
        <h2 className="text-base font-semibold">Keyboard shortcuts</h2>
        <button type="button" onClick={onClose} className="text-muted-foreground hover:text-foreground text-[13px]">
          Close (Esc)
        </button>
      </div>
      <ShortcutList pages={pages} />
    </Modal>
  );
}

export type PaletteItem = { id: string; label: string; hint?: string; run: () => void };

export function Palette({ items, onClose }: { items: PaletteItem[]; onClose: () => void }) {
  const [q, setQ] = useState("");
  const [at, setAt] = useState(0);
  const shown = items.filter((i) => i.label.toLowerCase().includes(q.trim().toLowerCase()));
  const pick = (i?: PaletteItem) => {
    if (!i) return;
    onClose();
    i.run();
  };
  return (
    <Modal label="Command palette" onClose={onClose}>
      <input
        ref={(el) => el?.focus()}
        value={q}
        placeholder="Go to a page, or find a shortcut"
        aria-label="Command"
        className="bg-background mb-2 w-full rounded-md border px-2 py-1.5 text-[13px] outline-none"
        onChange={(e) => {
          setQ(e.target.value);
          setAt(0);
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") {
            e.preventDefault();
            setAt((a) => Math.min(a + 1, shown.length - 1));
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            setAt((a) => Math.max(a - 1, 0));
          } else if (e.key === "Enter") pick(shown[at]);
        }}
      />
      {shown.length === 0 ? <p className="text-muted-foreground text-[13px]">Nothing matches.</p> : null}
      <ul aria-label="Commands">
        {shown.map((i, n) => (
          <li key={i.id}>
            <button
              type="button"
              aria-current={n === at ? "true" : undefined}
              onClick={() => pick(i)}
              onMouseMove={() => setAt(n)}
              className={cn(
                "flex w-full items-center justify-between rounded-md px-2 py-1.5 text-left text-[13px]",
                n === at && "bg-accent",
              )}
            >
              <span>{i.label}</span>
              {i.hint ? <kbd className={kbd}>{i.hint}</kbd> : null}
            </button>
          </li>
        ))}
      </ul>
    </Modal>
  );
}
