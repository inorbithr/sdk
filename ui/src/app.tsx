import { useEffect, useState } from "react";
import { extensionsMenu } from "./frame/extensions-menu.ts";
import { MenuList } from "./frame/menu-list.tsx";
import { PanelFrame } from "./frame/panel-frame.tsx";
import { type Listing, loadExtensions, type Panel } from "./lib/extensions.ts";
import { panelRoute, routedPanel, startPanel, stopPanel } from "./lib/panels.ts";

// The page `iohr ui serve` serves. The header and the stub body stand in for the Mac app's shell frame, which
// moves into this package in RFC 0217 C5; the body is labelled as a stub until then.
//
// A panel lives on its own page route, `/ext/<name>`: the server fixes a page's `frame-src` when it loads, so after a
// start the page loads again on that route and its policy allows the new panel's port. Close stops the panel and
// goes back to `/`.
export function App() {
  const [listing, setListing] = useState<Listing | null>(null);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [reason, setReason] = useState<string | null>(null);
  const routed = routedPanel(window.location.pathname);

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
  const chosen = routed === null ? null : (panels.find((p) => p.name === routed) ?? null);

  const start = async (panel: Panel) => {
    setBusy(true);
    setReason(null);
    const started = await startPanel(panel.name);
    if (started.state === "ok") {
      window.location.assign(panelRoute(panel.name));
      return;
    }
    setReason(started.reason);
    setBusy(false);
  };

  const close = async (panel: Panel) => {
    setBusy(true);
    await stopPanel(panel.name);
    window.location.assign("/");
  };

  const choose = (panel: Panel) => {
    if (panel.port !== undefined) {
      window.location.assign(panelRoute(panel.name));
      return;
    }
    void start(panel);
  };

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
              <MenuList label="Extensions" entries={extensionsMenu(panels, choose)} onClose={() => setOpen(false)} />
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
        {reason && !chosen ? (
          <p className="notice" role="alert">
            {reason}
          </p>
        ) : null}
        {chosen ? (
          <PanelFrame
            panel={chosen}
            busy={busy}
            reason={reason}
            onStart={() => void start(chosen)}
            onClose={() => void close(chosen)}
          />
        ) : routed !== null && listing?.state === "ok" ? (
          <p className="notice" role="alert">
            No installed extension named {routed} declares a panel.
          </p>
        ) : (
          <section className="frame-stub" data-stub="true" aria-label="Shell frame (stub)">
            <p className="stub-badge">Stub</p>
            <h1>The shell frame is not here yet</h1>
            <p>
              This page stands in for the Mac app's window frame (sidebar, headings, menus and the right pane), which
              moves into this package in a later slice of RFC 0217. The Extensions menu above lists the panels installed
              on this machine and starts the one you choose.
            </p>
          </section>
        )}
      </main>
    </div>
  );
}
