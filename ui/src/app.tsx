import { useEffect, useState } from "react";
import { extensionsMenu } from "./frame/extensions-menu.ts";
import { MenuList } from "./frame/menu-list.tsx";
import { PanelFrame } from "./frame/panel-frame.tsx";
import { type Listing, loadExtensions, type Panel } from "./lib/extensions.ts";

// The page `iohr ui serve` serves. The header and the stub body stand in for the Mac app's shell frame, which
// moves into this package in RFC 0217 C5; both are labelled as stubs until then.
export function App() {
  const [listing, setListing] = useState<Listing | null>(null);
  const [open, setOpen] = useState(false);
  const [chosen, setChosen] = useState<Panel | null>(null);

  useEffect(() => {
    let live = true;
    loadExtensions().then((l) => {
      if (live) setListing(l);
    });
    return () => {
      live = false;
    };
  }, []);

  const panels = listing?.state === "ok" ? listing.panels : [];

  return (
    <div className="app">
      <header className="bar">
        <span className="brand">InOrbit</span>
        <nav className="menubar" aria-label="Menu bar">
          <div className="menu-anchor">
            <button
              type="button"
              className="menubar-item"
              aria-haspopup="menu"
              aria-expanded={open}
              disabled={listing === null}
              onClick={() => setOpen((o) => !o)}
            >
              Extensions
            </button>
            {open ? (
              <MenuList label="Extensions" entries={extensionsMenu(panels, setChosen)} onClose={() => setOpen(false)} />
            ) : null}
          </div>
        </nav>
      </header>
      <main className="body">
        {listing?.state === "error" ? (
          <p className="notice" role="alert">
            {listing.reason}
          </p>
        ) : null}
        {chosen ? (
          <PanelFrame panel={chosen} />
        ) : (
          <section className="frame-stub" data-stub="true" aria-label="Shell frame (stub)">
            <p className="stub-badge">Stub</p>
            <h1>The shell frame is not here yet</h1>
            <p>
              This page stands in for the Mac app's window frame (sidebar, headings, menus and the right pane), which
              moves into this package in a later slice of RFC 0217. The Extensions menu above already reads the
              extensions installed on this machine.
            </p>
          </section>
        )}
      </main>
    </div>
  );
}
