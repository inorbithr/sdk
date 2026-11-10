import { House, PanelRight, Puzzle } from "lucide-react";
import { type MouseEvent, type ReactNode, useEffect, useRef, useState } from "react";
import { cn } from "./frame/cn.ts";
import { closeContextMenu, openContextMenu, showContextMenu } from "./frame/context-menu.tsx";
import type { MenuEntry } from "./frame/context-menu-core.ts";
import { Dock } from "./frame/dock.tsx";
import { closeDock, setDockWidth, toggleDock } from "./frame/dock-frame.ts";
import { extensionsMenu } from "./frame/extensions-menu.ts";
import { Frame } from "./frame/frame.tsx";
import { PanelFrame } from "./frame/panel-frame.tsx";
import { keysOf, useFrameShortcuts } from "./frame/shortcuts.ts";
import { toggleHidden } from "./frame/sidebar-frame.ts";
import { useDock } from "./lib/dock-store.ts";
import { type Listing, loadExtensions, type Panel } from "./lib/extensions.ts";
import { panelRoute, routedPanel, startPanel, stopPanel } from "./lib/panels.ts";
import { useSession } from "./lib/session-store.ts";
import { useSidebar } from "./lib/sidebar-store.ts";
import { SessionControl } from "./session-control.tsx";

// The page `iohr ui serve` serves, drawn in the shell frame from the Mac app (frame/frame.tsx, RFC 0217 C5): the
// sidebar lists Home and the installed extensions that declare a panel, the page header holds the Extensions menu,
// and the page body shows the chosen panel. The menus are the Mac app's one context menu (frame/context-menu.tsx,
// C6): the Extensions menu, and a right click on an extension row. ⌘\ hides and shows the sidebar, as in the Mac
// app. The Details button opens the right dock (frame/dock.tsx, C7), which shows what the listing says about the
// page in front: the installed panels on Home, the chosen extension on its own page.
//
// A panel lives on its own page route, `/ext/<name>`: the server fixes a page's `frame-src` when it loads, so after a
// start the page loads again on that route and its policy allows the new panel's port. Close stops the panel and
// goes back to `/`.
export function App() {
  const [sidebar, update] = useSidebar();
  const [dock, updateDock] = useDock();
  const account = useSession();
  const [listing, setListing] = useState<Listing | null>(null);
  const [open, setOpen] = useState(false);
  const menu = useRef<number | null>(null);
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

  useFrameShortcuts({ sidebar: () => update(toggleHidden) });

  const toggleMenu = (button: HTMLElement) => {
    if (open && menu.current !== null) {
      closeContextMenu(menu.current);
      return;
    }
    const r = button.getBoundingClientRect();
    setOpen(true);
    menu.current = openContextMenu({ x: r.right, y: r.bottom + 4 }, extensionsMenu(panels, choose), {
      label: "Extensions",
      trigger: button,
      onClose: () => {
        menu.current = null;
        setOpen(false);
      },
    });
  };

  const rowMenu = (p: Panel): MenuEntry[] => [
    {
      id: "open",
      label: p.port === undefined ? "Start" : "Open",
      icon: p.port === undefined ? "extension" : "panel",
      run: () => choose(p),
    },
    ...(p.port === undefined
      ? []
      : [{ id: "stop", label: "Stop", icon: "stop", danger: true, run: () => void close(p) }]),
  ];

  const title = (
    <h1 className="flex min-w-0 items-center gap-1.5 truncate text-sm font-medium">
      <span className={cn(chosen && "text-muted-foreground")}>InOrbit</span>
      {chosen ? (
        <>
          <span className="text-muted-foreground">/</span>
          <span className="truncate">{chosen.title}</span>
        </>
      ) : null}
    </h1>
  );

  const actions = (
    <>
      <SessionControl
        session={account.session}
        busy={account.busy}
        onSignIn={account.signIn}
        onSignOut={account.signOut}
      />
      <button
        type="button"
        className="hover:bg-accent hover:text-foreground text-muted-foreground rounded-md px-2 py-1 text-[13px] disabled:opacity-50"
        aria-haspopup="menu"
        aria-expanded={open}
        disabled={listing === null}
        onClick={(e) => toggleMenu(e.currentTarget)}
      >
        Extensions
      </button>
      <button
        type="button"
        title={dock.open ? "Hide details" : "Show details"}
        aria-label="Details"
        aria-pressed={dock.open}
        onClick={() => updateDock(toggleDock)}
        className={cn(
          "hover:bg-accent hover:text-foreground text-muted-foreground flex size-7 items-center justify-center rounded-md",
          dock.open && "bg-accent text-foreground",
        )}
      >
        <PanelRight className="size-4" />
      </button>
    </>
  );

  const sidebarBody = (rail: boolean) => (
    <nav aria-label="Pages" className={cn("flex flex-col gap-3 pb-2", rail ? "items-center px-2" : "px-2")}>
      <ul className={cn("flex flex-col gap-0.5", rail && "items-center")}>
        <NavItem
          rail={rail}
          label="Home"
          icon={<House />}
          active={routed === null}
          onClick={() => window.location.assign("/")}
        />
      </ul>
      <section aria-label="Extensions" className={cn("flex flex-col gap-0.5", rail && "items-center")}>
        {!rail && <h2 className="text-muted-foreground px-2 pb-0.5 text-[11px] font-medium">Extensions</h2>}
        <ul className={cn("flex flex-col gap-0.5", rail && "items-center")}>
          {panels.map((p) => (
            <NavItem
              key={p.name}
              rail={rail}
              label={p.title}
              hint={p.port === undefined ? undefined : "running"}
              icon={<Puzzle />}
              active={chosen?.name === p.name}
              onClick={() => choose(p)}
              onContextMenu={(e) => showContextMenu(e, rowMenu(p), { label: p.title })}
            />
          ))}
          {listing?.state === "ok" && panels.length === 0 && !rail ? (
            <li className="text-muted-foreground px-2 py-0.5 text-[13px]">No installed extension declares a panel.</li>
          ) : null}
        </ul>
      </section>
    </nav>
  );

  return (
    <Frame
      sidebar={sidebar}
      update={update}
      title={title}
      actions={actions}
      sidebarBody={sidebarBody}
      dock={
        dock.open ? (
          <Dock
            title="Details"
            width={dock.width}
            onWidth={(w) => updateDock((s) => setDockWidth(s, w))}
            onClose={() => updateDock(closeDock)}
          >
            <Details listing={listing} chosen={chosen} />
          </Dock>
        ) : null
      }
    >
      <div className={cn("h-full p-5", chosen && "flex flex-col")}>
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
          <section className="frame-stub" data-stub="true" aria-label="Home (stub)">
            <p className="stub-badge">Stub</p>
            <h2 className="mb-2 text-base font-semibold">Home</h2>
            <p className="text-muted-foreground text-sm">
              This is the Mac app's window frame: the sidebar, the page header and its context menu. Right-click an
              extension for its actions, press {keysOf("sidebar")} to hide or show the sidebar, and open the dock on the
              right with Details. Choose an extension in the sidebar or the Extensions menu to start its panel.
            </p>
          </section>
        )}
      </div>
    </Frame>
  );
}

/** The dock's Details: what the extension listing says about the page in front, nothing made up. */
function Details({ listing, chosen }: { listing: Listing | null; chosen: Panel | null }) {
  if (listing === null) return <p className="text-muted-foreground p-4 text-[13px]">Loading the extensions…</p>;
  if (listing.state === "error")
    return (
      <p className="text-muted-foreground p-4 text-[13px]" role="alert">
        {listing.reason}
      </p>
    );
  const rows = chosen ? [chosen] : listing.panels;
  if (rows.length === 0)
    return <p className="text-muted-foreground p-4 text-[13px]">No installed extension declares a panel.</p>;
  return (
    <div className="flex flex-col gap-3 p-4 text-[13px]">
      {!chosen && <p className="text-muted-foreground">Installed extensions that declare a panel.</p>}
      {rows.map((p) => (
        <dl key={p.name} className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 rounded-lg border p-3">
          <dt className="text-muted-foreground">Name</dt>
          <dd className="truncate">{p.name}</dd>
          <dt className="text-muted-foreground">Title</dt>
          <dd className="truncate">{p.title}</dd>
          <dt className="text-muted-foreground">Version</dt>
          <dd className="truncate">{p.version}</dd>
          <dt className="text-muted-foreground">Entry</dt>
          <dd className="truncate font-mono text-[12px]">{p.entry}</dd>
          <dt className="text-muted-foreground">State</dt>
          <dd>{p.port === undefined ? "stopped" : "running"}</dd>
        </dl>
      ))}
    </div>
  );
}

/** One sidebar row, after the Mac app's Row (expanded) and RailButton (rail). */
function NavItem({
  rail,
  label,
  hint,
  icon,
  active,
  onClick,
  onContextMenu,
}: {
  rail: boolean;
  label: string;
  hint?: string;
  icon: ReactNode;
  active: boolean;
  onClick: () => void;
  onContextMenu?: (e: MouseEvent) => void;
}) {
  if (rail)
    return (
      <li>
        <button
          type="button"
          aria-label={label}
          title={label}
          aria-current={active ? "page" : undefined}
          onClick={onClick}
          onContextMenu={onContextMenu}
          className={cn(
            "ring-sidebar-ring text-muted-foreground hover:bg-sidebar-accent hover:text-foreground relative flex size-10 shrink-0 items-center justify-center rounded-lg outline-none transition-colors focus-visible:ring-2 [&>svg]:size-[18px]",
            active && "bg-sidebar-accent text-foreground",
          )}
        >
          {icon}
        </button>
      </li>
    );
  return (
    <li>
      <button
        type="button"
        aria-current={active ? "page" : undefined}
        onClick={onClick}
        onContextMenu={onContextMenu}
        className={cn(
          "ring-sidebar-ring flex min-h-7 w-full items-center gap-2 rounded-md px-2 py-0.5 text-left text-[13px] outline-none transition-colors focus-visible:ring-2 [&>svg]:size-3.5 [&>svg]:shrink-0",
          active ? "bg-sidebar-accent text-sidebar-accent-foreground font-medium" : "hover:bg-sidebar-accent/70",
        )}
      >
        {icon}
        <span className="min-w-0 flex-1 truncate">{label}</span>
        {hint ? <span className="text-muted-foreground text-[11px]">{hint}</span> : null}
      </button>
    </li>
  );
}
