import {
  ArrowLeftRight,
  Atom,
  House,
  LayoutGrid,
  Orbit,
  PanelRight,
  PenTool,
  Puzzle,
  Settings as SettingsIcon,
  Wrench,
} from "lucide-react";
import { lazy, type MouseEvent, type ReactNode, Suspense, useEffect, useRef, useState } from "react";
import { CANVAS, canvasRoute } from "./canvas/canvas-route.ts";
import {
  applyLayout,
  applySpan,
  arrangeBox,
  back,
  dockClosed,
  type LayoutId,
  opensDock,
  type Place,
  resetAll,
  restore,
} from "./frame/arrange.ts";
import { arrangeMenu, layoutMenu } from "./frame/arrange-menu.ts";
import { cn } from "./frame/cn.ts";
import { closeContextMenu, openContextMenu, showContextMenu } from "./frame/context-menu.tsx";
import type { MenuEntry } from "./frame/context-menu-core.ts";
import { Dock } from "./frame/dock.tsx";
import { closeDock, openDock, setDockWidth, toggleDock } from "./frame/dock-frame.ts";
import { extensionsMenu } from "./frame/extensions-menu.ts";
import { ARRANGE_MIN_WIDTH, Frame, useViewportWidth } from "./frame/frame.tsx";
import { GridPicker } from "./frame/grid-picker.tsx";
import { PanelFrame } from "./frame/panel-frame.tsx";
import { toggleHidden } from "./frame/sidebar-frame.ts";
import { Home } from "./home.tsx";
import { accountMenu, accountName } from "./lib/accounts-core.ts";
import { useAccounts } from "./lib/accounts-store.ts";
import { updateArrangement as updateArr, useArrangement } from "./lib/arrange-store.ts";
import { useDock } from "./lib/dock-store.ts";
import { type Listing, loadExtensions, type Panel } from "./lib/extensions.ts";
import { entryOf, isDown, whyClosed } from "./lib/features-core.ts";
import { allOnline, backOnline, putDown, undoLast, useFeatures, useUndo } from "./lib/features-store.ts";
import type { Space } from "./lib/home-core.ts";
import { homeReads } from "./lib/home-data.ts";
import { add as addOrbit, HOME, remove as removeOrbit } from "./lib/orbits-core.ts";
import { updateOrbits, useOrbits } from "./lib/orbits-store.ts";
import { maintenanceMenu, orbitsMenu } from "./lib/page-menus.ts";
import { panelRoute, routedPanel, startPanel, stopPanel } from "./lib/panels.ts";
import { useSession } from "./lib/session-store.ts";
import { SETTINGS, settingsRoute } from "./lib/settings-core.ts";
import { useRead } from "./lib/settings-data.ts";
import { BINDINGS } from "./lib/shortcut-tables.ts";
import { useSidebar } from "./lib/sidebar-store.ts";
import { useShortcuts } from "./lib/use-shortcuts.ts";
import { MaintenanceForm } from "./maintenance-form.tsx";
import { RailAccount, SessionControl } from "./session-control.tsx";
import { Settings } from "./settings.tsx";
import { HelpOverlay, keysShown, Palette, type PaletteItem } from "./shortcuts-view.tsx";

// The Workload page (3D orbit, room, tree): its own chunk, so three loads only on /workload.
const Workload = lazy(() => import("./workload.tsx"));
const WORKLOAD = "/workload";

const Canvas = lazy(() => import("./canvas/canvas-page.tsx"));

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
  const raw = useSession();
  const accounts = useAccounts(raw.session);
  const account = { ...raw, session: accounts.session };
  const spaces = useRead<{ spaces?: Space[] }, Space[]>(
    accounts.active === null ? null : homeReads(accounts.active).spaces,
    (w) => w.spaces ?? [],
  );
  const accountLabel = accountName(accounts.rows, accounts.active);
  const [listing, setListing] = useState<Listing | null>(null);
  const [open, setOpen] = useState(false);
  const menu = useRef<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [reason, setReason] = useState<string | null>(null);
  const routed = routedPanel(window.location.pathname);
  const arr = useArrangement();
  const orbits = useOrbits();
  const features = useFeatures();
  const undo = useUndo();
  const wide = useViewportWidth() >= ARRANGE_MIN_WIDTH;
  const [gridAt, setGridAt] = useState<HTMLElement | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const layoutButton = useRef<HTMLButtonElement>(null);
  const inSettings = settingsRoute(window.location.pathname) !== null;
  const inCanvas = canvasRoute(window.location.pathname) !== null;
  const inWorkload = window.location.pathname === WORKLOAD;
  const pageKey = inSettings
    ? "settings"
    : inCanvas
      ? "canvas"
      : inWorkload
        ? "workload"
        : routed === null
          ? HOME
          : `ext:${routed}`;

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

  const extensionsDown = isDown(entryOf(features, "extensions"));
  const homeDown = isDown(entryOf(features, "home"));

  const choose = (panel: Panel) => {
    if (extensionsDown) {
      setReason(`Extension panels: ${whyClosed(entryOf(features, "extensions"))}`);
      return;
    }
    if (panel.port !== undefined) {
      window.location.assign(panelRoute(panel.name));
      return;
    }
    void start(panel);
  };

  // A pair layout shows the dock to put beside the page; a dock that was closed ends it; a stored layout that has a dock
  // place opens the dock again, so what the layout says and what is drawn never part.
  const pick = (id: LayoutId) => {
    updateArr((a) => applyLayout(a, id));
    if (opensDock(id)) updateDock(openDock);
  };
  const pickSpan = (p: Place) => updateArr((a) => applySpan(a, p));
  useEffect(() => {
    if (arr.dock !== null && !dock.open) updateDock(openDock);
  }, [arr.dock, dock.open, updateDock]);
  const closeTheDock = () => {
    updateDock(closeDock);
    updateArr(dockClosed);
  };
  const openGrid = () => setGridAt((at) => (at ? null : layoutButton.current));
  const layoutActions = { layout: pick, grid: openGrid, back: () => updateArr(back) };

  const [help, setHelp] = useState(false);
  const [palette, setPalette] = useState(false);
  const toggleHelp = () => setHelp((h) => !h);
  useShortcuts("global", {
    sidebar: () => update(toggleHidden),
    palette: () => setPalette((p) => !p),
    shortcuts: toggleHelp,
    "shortcuts-help": toggleHelp,
    "go-home": () => goTo(HOME),
    "go-settings": () => goTo("settings"),
    ...(wide
      ? {
          "layout-left": () => pick("left"),
          "layout-right": () => pick("right"),
          "layout-top": () => pick("top"),
          "layout-bottom": () => pick("bottom"),
          "layout-full": () => pick("full"),
          "layout-grid": openGrid,
          "layout-back": () => updateArr(back),
        }
      : {}),
  });

  const menuAt = (button: HTMLElement, entries: MenuEntry[], label: string, align: "right" | "left" = "right") => {
    const r = button.getBoundingClientRect();
    openContextMenu({ x: align === "right" ? r.right : r.left, y: r.bottom + 4 }, entries, { label, trigger: button });
  };

  // The account switcher (RFC 0217.1): accounts, projects and the environment, as the Mac app's popover.
  const openSwitcher = (button: HTMLElement) =>
    menuAt(
      button,
      accountMenu({
        rows: accounts.rows,
        active: accounts.active,
        projects: spaces.state === "ok" ? spaces.data.map((x) => ({ id: x.space_id, name: x.name })) : [],
        profile: account.session?.state === "signed-in" ? account.session.profile : "",
        signedIn: account.session?.state === "signed-in",
        switchTo: accounts.switchTo,
        signIn: account.signIn,
        signOut: account.signOut,
      }),
      accountLabel,
      "left",
    );

  const pages = [
    { key: HOME, title: "Home" },
    { key: "canvas", title: "Canvas" },
    { key: "workload", title: "Workload" },
    { key: "settings", title: "Settings" },
    ...panels.map((p) => ({ key: `ext:${p.name}`, title: p.title })),
  ];
  const goTo = (key: string) => {
    if (key === pageKey) return;
    if (key === HOME) {
      window.location.assign("/");
      return;
    }
    if (key === "settings") {
      window.location.assign(SETTINGS);
      return;
    }
    if (key === "canvas") {
      window.location.assign(CANVAS);
      return;
    }
    if (key === "workload") {
      window.location.assign(WORKLOAD);
      return;
    }
    const panel = panels.find((p) => `ext:${p.name}` === key);
    if (panel) choose(panel);
  };
  const pageIds = ["home", "settings"];
  const paletteItems: PaletteItem[] = [
    ...pages.map((p) => {
      const go = BINDINGS.find((b) => b.page === (p.key === HOME ? "home" : p.key));
      return {
        id: `go:${p.key}`,
        label: `Go to ${p.title}`,
        hint: go ? keysShown(go) : undefined,
        run: () => goTo(p.key),
      };
    }),
    { id: "help", label: "Keyboard shortcuts", hint: "?", run: () => setHelp(true) },
  ];
  const orbitActions = {
    go: goTo,
    add: (k: string) => updateOrbits((o) => addOrbit(o, k)),
    remove: (k: string) => updateOrbits((o) => removeOrbit(o, k)),
  };
  const maintenanceActions = {
    edit: (id: string) => setEditing(id),
    back: backOnline,
    allOnline,
    undo: undo ? undoLast : null,
    undoLabel: undo?.label ?? "",
  };
  const arrangeBy = (id: "page" | "dock") => (button: HTMLElement) =>
    menuAt(
      button,
      arrangeMenu(arr, id, {
        move: (to) => updateArr((a) => arrangeBox(a, id, to)),
        reset: () => updateArr(resetAll),
      }),
      "Arrange",
    );

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
      <span className={cn((chosen || inSettings || inWorkload || inCanvas) && "text-muted-foreground")}>InOrbit</span>
      {chosen || inSettings || inWorkload || inCanvas ? (
        <>
          <span className="text-muted-foreground">/</span>
          <span className="truncate">
            {inSettings ? "Settings" : inCanvas ? "Canvas" : inWorkload ? "Workload" : chosen?.title}
          </span>
        </>
      ) : null}
    </h1>
  );

  const anyDown = isDown(entryOf(features, "home")) || isDown(entryOf(features, "extensions"));
  const headerButton =
    "hover:bg-accent hover:text-foreground text-muted-foreground flex items-center gap-1.5 whitespace-nowrap rounded-md px-2 py-1 text-[13px] [&>svg]:size-3.5";

  const actions = (
    <>
      <button
        type="button"
        title="Maintenance"
        aria-label="Maintenance"
        aria-haspopup="menu"
        data-maintenance-button
        className={cn(headerButton, "size-7 justify-center px-0", anyDown && "bg-accent text-foreground")}
        onClick={(e) => menuAt(e.currentTarget, maintenanceMenu(features, maintenanceActions), "Maintenance")}
      >
        <Wrench />
      </button>
      <button
        type="button"
        aria-haspopup="menu"
        data-orbits-button
        className={headerButton}
        onClick={(e) => menuAt(e.currentTarget, orbitsMenu(orbits, pageKey, pages, orbitActions), "Orbits")}
      >
        <Orbit aria-hidden="true" />
        <span className="@max-[640px]:hidden">Orbits</span>
      </button>
      {wide && (
        <>
          <button
            ref={layoutButton}
            type="button"
            title="Window layout"
            aria-label="Layout"
            aria-haspopup="menu"
            data-layout-button
            className={cn(headerButton, "size-7 justify-center px-0")}
            onClick={(e) => menuAt(e.currentTarget, layoutMenu(arr, layoutActions), "Layout")}
          >
            <LayoutGrid />
          </button>
          <button
            type="button"
            className={headerButton}
            aria-haspopup="menu"
            data-arrange-button="page"
            aria-label="Arrange"
            onClick={(e) => arrangeBy("page")(e.currentTarget)}
          >
            <span className="@max-[640px]:hidden">Arrange</span>
            <ArrowLeftRight aria-hidden="true" className="hidden @max-[640px]:block" />
          </button>
        </>
      )}
      <SessionControl
        session={account.session}
        name={accountLabel}
        busy={account.busy}
        onSignIn={account.signIn}
        onOpen={openSwitcher}
      />
      <button
        type="button"
        className="hover:bg-accent hover:text-foreground text-muted-foreground rounded-md px-2 py-1 text-[13px] disabled:opacity-50"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label="Extensions"
        disabled={listing === null}
        onClick={(e) => toggleMenu(e.currentTarget)}
      >
        <span className="@max-[640px]:hidden">Extensions</span>
        <Puzzle aria-hidden="true" className="hidden size-4 @max-[640px]:block" />
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
          active={routed === null && !inSettings && !inWorkload && !inCanvas}
          onClick={() => window.location.assign("/")}
        />
        <NavItem rail={rail} label="Canvas" icon={<PenTool />} active={inCanvas} onClick={() => goTo("canvas")} />
        <NavItem rail={rail} label="Workload" icon={<Atom />} active={inWorkload} onClick={() => goTo("workload")} />
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
    <>
      <Frame
        sidebar={sidebar}
        update={update}
        title={title}
        actions={actions}
        sidebarBody={sidebarBody}
        sidebarFooter={(rail) => (
          <div className={cn("flex flex-col gap-2", rail && "items-center")}>
            <ul>
              <NavItem
                rail={rail}
                label="Settings"
                icon={<SettingsIcon />}
                active={inSettings}
                onClick={() => goTo("settings")}
              />
            </ul>
            <RailAccount session={account.session} name={accountLabel} rail={rail} onOpen={openSwitcher} />
          </div>
        )}
        arrangement={arr}
        onRestore={() => updateArr(restore)}
        dock={(slot) =>
          dock.open ? (
            <Dock
              title="Details"
              width={dock.width}
              onWidth={(w) => updateDock((s) => setDockWidth(s, w))}
              onClose={closeTheDock}
              side={slot.side}
              fill={slot.fill}
              headerActions={
                wide ? (
                  <button
                    type="button"
                    className={headerButton}
                    aria-haspopup="menu"
                    data-arrange-button="dock"
                    onClick={(e) => arrangeBy("dock")(e.currentTarget)}
                  >
                    Arrange
                  </button>
                ) : null
              }
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
          {inWorkload ? (
            <Suspense fallback={<p className="text-muted-foreground text-sm">Loading the workload…</p>}>
              <Workload />
            </Suspense>
          ) : inSettings ? (
            <Settings
              session={account.session}
              accounts={accounts}
              busy={account.busy}
              onSignIn={account.signIn}
              onSignOut={account.signOut}
              listing={listing}
              onEdit={setEditing}
            />
          ) : inCanvas ? (
            <Suspense fallback={<p className="text-muted-foreground text-sm">Opening the canvas…</p>}>
              <Canvas />
            </Suspense>
          ) : (routed !== null && extensionsDown) || (routed === null && homeDown) ? (
            <section className="notice" role="status" data-maintenance-notice>
              <h2 className="mb-1 text-base font-semibold">
                {routed === null ? "Home" : "Extension panels"} is under maintenance
              </h2>
              <p className="text-sm">{whyClosed(entryOf(features, routed === null ? "home" : "extensions"))}</p>
              {entryOf(features, routed === null ? "home" : "extensions").eta !== "" && (
                <p className="text-muted-foreground text-sm">
                  Back at {entryOf(features, routed === null ? "home" : "extensions").eta}
                </p>
              )}
              <p className="text-muted-foreground mt-2 text-[12px]">
                Kept in this browser. Bring it back from the wrench at the top.
              </p>
            </section>
          ) : chosen ? (
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
            <Home session={account.session} busy={account.busy} onSignIn={account.signIn} />
          )}
        </div>
      </Frame>
      {help ? <HelpOverlay pages={pageIds} onClose={() => setHelp(false)} /> : null}
      {palette ? <Palette items={paletteItems} onClose={() => setPalette(false)} /> : null}
      {gridAt ? <GridPicker at={gridAt} onPick={pickSpan} onClose={() => setGridAt(null)} /> : null}
      {editing ? (
        <MaintenanceForm id={editing} onSave={(why) => putDown(editing, why)} onClose={() => setEditing(null)} />
      ) : null}
    </>
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
