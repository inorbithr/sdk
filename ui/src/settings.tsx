import { Search } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { cn } from "./frame/cn.ts";
import { DEFAULT_PANE, matchPanes, type Pane, type PaneId, paneById, paneHref, SETTINGS } from "./lib/settings-core.ts";
import {
  About,
  Accounts,
  Appearance,
  Developer,
  Features,
  General,
  Keyboard,
  NotHere,
  type PaneProps,
  Privacy,
  Projects,
  Team,
} from "./settings-panes.tsx";

// Settings (RFC 0217, slice 4): the Mac app's Settings as a list of panes at the left and the chosen pane at the
// right, in the Mac app's order and names. Every entry opens a page: the ones this page can do work, the rest say
// where they work. A pane is its own address, `/settings/<pane>`; choosing one changes the address without loading
// the page again, and Back goes to the pane before. The search filters the list and Escape clears it.

const BODY: Partial<Record<PaneId, (p: PaneProps) => ReactNode>> = {
  general: () => <General />,
  appearance: () => <Appearance />,
  accounts: (p) => <Accounts {...p} />,
  team: (p) => <Team {...p} />,
  projects: (p) => <Projects {...p} />,
  keyboard: () => <Keyboard />,
  features: (p) => <Features {...p} />,
  privacy: () => <Privacy />,
  developer: (p) => <Developer {...p} />,
  about: (p) => <About {...p} />,
};

/** The pane the address names: a pane, or null for a name that is no pane. */
const paneAt = (pathname: string): Pane | null => {
  const m = pathname.match(/^\/settings(?:\/([^/]*))?\/?$/);
  if (!m) return null;
  return paneById(m[1] || DEFAULT_PANE) ?? null;
};

export function Settings(props: PaneProps) {
  const [at, setAt] = useState(window.location.pathname);
  const [query, setQuery] = useState("");
  useEffect(() => {
    const on = () => setAt(window.location.pathname);
    window.addEventListener("popstate", on);
    return () => window.removeEventListener("popstate", on);
  }, []);
  const pane = paneAt(at);
  const open = (id: PaneId) => {
    const to = paneHref(id);
    if (window.location.pathname !== to) window.history.pushState(null, "", to);
    setAt(to);
  };
  const shown = matchPanes(query);
  const input =
    "bg-background w-full rounded-md border py-1.5 pr-2 pl-8 text-[13px] outline-none focus-visible:ring-2 ring-ring";

  return (
    <div className="@container mx-auto flex max-w-4xl flex-col gap-4" data-settings>
      <h1 className="text-lg font-semibold">Settings</h1>
      <div className="flex gap-6 @max-[640px]:flex-col">
        <nav aria-label="Settings panes" className="w-56 shrink-0 @max-[640px]:w-full">
          <div className="relative mb-2">
            <Search aria-hidden="true" className="text-muted-foreground absolute top-2 left-2.5 size-3.5" />
            <input
              type="search"
              aria-label="Search settings"
              placeholder="Search"
              className={input}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") setQuery("");
              }}
            />
          </div>
          <ul className="flex flex-col gap-0.5 @max-[640px]:hidden">
            {shown.map((p) => (
              <li key={p.id}>
                <button
                  type="button"
                  data-pane={p.id}
                  aria-current={pane?.id === p.id ? "page" : undefined}
                  onClick={() => open(p.id)}
                  className={cn(
                    "ring-sidebar-ring flex min-h-7 w-full items-center gap-2 rounded-md px-2 py-0.5 text-left text-[13px] outline-none focus-visible:ring-2",
                    pane?.id === p.id ? "bg-accent text-foreground font-medium" : "hover:bg-accent/70",
                    !p.built && "text-muted-foreground",
                  )}
                >
                  <span className="min-w-0 flex-1 truncate">{p.label}</span>
                </button>
              </li>
            ))}
            {shown.length === 0 ? (
              <li className="text-muted-foreground px-2 py-1 text-[13px]" data-no-match>
                No setting matches “{query.trim()}”.{" "}
                <button type="button" className="underline" onClick={() => setQuery("")}>
                  Clear search
                </button>
              </li>
            ) : null}
          </ul>
          <select
            aria-label="Pane"
            className={cn(input, "hidden pl-2 @max-[640px]:block")}
            value={pane?.id ?? ""}
            onChange={(e) => open(e.target.value as PaneId)}
          >
            {pane === null ? <option value="">Choose a pane</option> : null}
            {shown.map((p) => (
              <option key={p.id} value={p.id}>
                {p.label}
              </option>
            ))}
          </select>
        </nav>
        <main
          className="flex min-w-0 flex-1 flex-col gap-3"
          aria-label={pane?.label ?? "Settings"}
          data-pane-body={pane?.id ?? "unknown"}
        >
          {pane === null ? (
            <section className="notice" role="alert">
              <h2 className="text-base font-semibold">There is no such setting</h2>
              <p className="text-sm">
                <a
                  className="underline"
                  href={SETTINGS}
                  onClick={(e) => {
                    e.preventDefault();
                    open(DEFAULT_PANE);
                  }}
                >
                  Open General
                </a>
              </p>
            </section>
          ) : (
            <>
              <header>
                <h2 className="text-base font-semibold">{pane.label}</h2>
                <p className="text-muted-foreground text-[13px]">{pane.blurb}</p>
              </header>
              {BODY[pane.id]?.(props) ?? <NotHere pane={pane} />}
            </>
          )}
        </main>
      </div>
    </div>
  );
}
