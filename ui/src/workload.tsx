import { lazy, Suspense, useEffect, useMemo, useState } from "react";
import { cn } from "./frame/cn.ts";
import { apiGet, apiSend } from "./lib/api.ts";
import { type LiveState, useLive } from "./lib/live.ts";
import { moonOffset, moonRadius, planetAt, planetRadius, sceneExtent } from "./lib/orbit-layout.ts";
import { applyFrame, emptyPicture, type Picture } from "./lib/picture.ts";
import { actionsFor, type WorkAction } from "./lib/workload-actions.ts";
import {
  agentsOf,
  blocked,
  buildMap,
  layerCounts,
  type NodeState,
  orbitPlanets,
  pathTo,
  roomTiles,
  STATE_COLOR,
  STATE_WORD,
  STATES,
  treeRows,
  type WNode,
  type WorkloadMap,
  type WorkPlanet,
} from "./lib/workload-map.ts";
import { hasWebGL, useReducedMotion } from "./scene/capabilities.ts";

// The Workload page (RFC 0217): everything that works for the account as an orbit, a room of tiles and a
// tree, on the same picture. The 3D scene is its own chunk and loads only with the orbit view; without
// WebGL the orbit is drawn flat from the same layout. Jobs, runners, asks and waiting items move live over
// the coordinator's picture stream; the agent list is read every 30 s because the API has no stream for it.
const OrbitScene = lazy(() => import("./scene/orbit-scene.tsx"));

type View = "orbit" | "room" | "tree";
const VIEWS: { id: View; label: string }[] = [
  { id: "orbit", label: "Orbit" },
  { id: "room", label: "Room" },
  { id: "tree", label: "Tree" },
];
const AGENTS_EVERY_MS = 30_000;
const LIVE_WORD: Record<LiveState, string> = {
  connecting: "Connecting",
  live: "Live",
  reconnecting: "Reconnecting",
  offline: "Not live",
};

function useAgents(): { agents: ReturnType<typeof agentsOf>; note: string | null } {
  const [agents, setAgents] = useState<ReturnType<typeof agentsOf>>([]);
  const [note, setNote] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    const read = async () => {
      if (document.hidden) return;
      const r = await apiGet("/v1/agents");
      if (!live) return;
      if (r.state === "ok") {
        setAgents(agentsOf(r.data));
        setNote(null);
      } else setNote(r.state === "signed-out" ? "Sign in to see agents." : r.reason);
    };
    void read();
    const t = setInterval(() => void read(), AGENTS_EVERY_MS);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, []);
  return { agents, note };
}

const Dot = ({ state }: { state: NodeState }) => (
  <span
    aria-hidden
    className="inline-block size-2 shrink-0 rounded-full"
    style={{ background: `var(${STATE_COLOR_VAR[state]}, ${STATE_COLOR[state]})` }}
  />
);
const STATE_COLOR_VAR: Record<NodeState, string> = {
  working: "--success",
  needs: "--warning",
  idle: "--muted-foreground",
  stuck: "--warning",
  failed: "--destructive",
  done: "--muted-foreground",
};

/** What makes a drawn shape a button: focusable, Enter and Space pick it. */
const pickable = (id: string, onPick: (id: string) => void) => ({
  role: "button" as const,
  tabIndex: 0,
  onClick: () => onPick(id),
  onKeyDown: (e: { key: string; preventDefault: () => void }) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onPick(id);
    }
  },
});

/** The orbit drawn flat: the same layout as the 3D scene, for a browser without WebGL. */
function OrbitFlat({
  planets,
  picked,
  onPick,
}: {
  planets: WorkPlanet[];
  picked: string | null;
  onPick: (id: string) => void;
}) {
  const e = sceneExtent(planets);
  return (
    <svg
      viewBox={`${-e} ${-e * 0.6} ${e * 2} ${e * 1.2}`}
      className="h-full min-h-[320px] w-full"
      role="img"
      aria-label={`Workload orbit, flat: ${planets.map((p) => p.say).join(" ")}`}
    >
      {planets.map((p, i) => {
        const c = planetAt(i, planets.length);
        return (
          <g key={p.id} transform={`translate(${c.x} ${c.z})`}>
            {[...new Set(p.moons.map((_, k) => Math.floor(k / 8)))].map((ring) => (
              <circle
                key={ring}
                r={moonRadius(p, ring * 8)}
                fill="none"
                stroke="currentColor"
                opacity={0.2}
                strokeWidth={0.02}
              />
            ))}
            <circle
              r={planetRadius(p)}
              fill={STATE_COLOR[p.tone]}
              stroke={picked === p.id ? "currentColor" : "none"}
              strokeWidth={0.06}
              {...pickable(p.id, onPick)}
              className="cursor-pointer"
            >
              <title>{p.say}</title>
            </circle>
            <text
              y={planetRadius(p) + moonRadius(p, p.moons.length) + 0.3}
              textAnchor="middle"
              fontSize={0.32}
              fill="currentColor"
            >
              {p.name}
            </text>
            {p.moons.map((m, k) => {
              const o = moonOffset(p, m, k, 0);
              return (
                <circle
                  key={m.id}
                  cx={o.x}
                  cy={o.z}
                  r={picked === m.id ? 0.2 : 0.12}
                  fill={STATE_COLOR[m.tone]}
                  {...pickable(m.id, onPick)}
                  className="cursor-pointer"
                >
                  <title>{`${m.name}: ${m.say}`}</title>
                </circle>
              );
            })}
          </g>
        );
      })}
    </svg>
  );
}

function NodePanel({
  map,
  id,
  onClose,
  onDone,
}: {
  map: WorkloadMap;
  id: string;
  onClose: () => void;
  onDone: (m: string) => void;
}) {
  const n = map.nodes.get(id);
  const [busy, setBusy] = useState(false);
  const [say, setSay] = useState<string | null>(null);
  if (!n) return null;
  const act = async (a: WorkAction) => {
    if (!window.confirm(a.confirm)) return;
    setBusy(true);
    const r = await apiSend(a.method, a.path);
    setBusy(false);
    setSay(
      r.state === "ok"
        ? `${a.label}: sent.`
        : `${a.label} failed: ${r.state === "error" ? r.reason : "You are signed out."}`,
    );
    if (r.state === "ok") onDone(`${a.label} sent for ${n.label}.`);
  };
  return (
    <aside aria-label="Selected" className="rounded-md border p-3 text-sm">
      <div className="flex items-start gap-2">
        <Dot state={n.state} />
        <div className="min-w-0 flex-1">
          <h3 className="truncate font-medium">{n.label}</h3>
          <p className="text-muted-foreground text-[12px]">
            {[n.kind, STATE_WORD[n.state], n.sub].filter(Boolean).join(" · ")}
          </p>
        </div>
        <button type="button" className="text-muted-foreground text-[12px]" onClick={onClose}>
          Show all
        </button>
      </div>
      <p className="text-muted-foreground mt-2 text-[12px]">
        {pathTo(map, id)
          .map((p) => map.nodes.get(p)?.label ?? p)
          .join(" › ")}
      </p>
      {n.latest ? <p className="mt-1">{n.latest}</p> : null}
      {n.owner ? <p className="text-muted-foreground text-[12px]">Held by {n.owner}</p> : null}
      {n.stub ? (
        <p className="text-warning mt-1 text-[12px]">Stub: a fixture stands in for a source not wired yet.</p>
      ) : null}
      <div className="mt-2 flex gap-2">
        {actionsFor(n).map((a) => (
          <button
            key={a.id}
            type="button"
            disabled={busy}
            className="rounded border px-2 py-0.5 text-[12px] disabled:opacity-50"
            onClick={() => void act(a)}
          >
            {a.label}
          </button>
        ))}
      </div>
      {say ? (
        <p className="mt-1 text-[12px]" role="status">
          {say}
        </p>
      ) : null}
    </aside>
  );
}

export default function Workload() {
  const [view, setView] = useState<View>("orbit");
  const [picked, setPicked] = useState<string | null>(null);
  const [only, setOnly] = useState<NodeState | null>(null);
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
  const [toast, setToast] = useState<string | null>(null);
  const reduced = useReducedMotion();
  const webgl = useMemo(() => hasWebGL(), []);
  const { agents, note } = useAgents();
  const live = useLive<unknown, Picture>("/v1/coordinator/picture/events", {
    reduce: (p, e) => applyFrame(p ?? emptyPicture(), e),
    initial: emptyPicture(),
  });
  const picture = live.value ?? emptyPicture();
  const map = useMemo(() => buildMap(picture, agents), [picture, agents]);
  const counts = layerCounts(map);
  const held = useMemo(() => blocked(map), [map]);
  const planets = useMemo(() => {
    if (!only) return orbitPlanets(map);
    return orbitPlanets(map).map((p) => ({ ...p, moons: p.moons.filter((m) => m.tone === only) }));
  }, [map, only]);
  const tiles = useMemo(() => roomTiles(map).filter((t) => !only || t.state === only), [map, only]);
  const rows = useMemo(() => treeRows(map, collapsed, only ? new Set([only]) : undefined), [map, collapsed, only]);

  useEffect(() => {
    const esc = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      setPicked(null);
      setOnly(null);
    };
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, []);
  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 4000);
    return () => clearTimeout(t);
  }, [toast]);

  const empty = map.nodes.size === 3;
  return (
    <section aria-label="Explore" className="flex min-h-0 flex-1 flex-col gap-3">
      <header className="flex flex-wrap items-center gap-3">
        <div role="tablist" aria-label="Workload view" className="flex gap-1">
          {VIEWS.map((v) => (
            <button
              key={v.id}
              type="button"
              role="tab"
              aria-selected={view === v.id}
              className={cn(
                "rounded px-2 py-0.5 text-sm",
                view === v.id ? "bg-accent font-medium" : "text-muted-foreground",
              )}
              onClick={() => setView(v.id)}
            >
              {v.label}
            </button>
          ))}
        </div>
        <ul className="flex flex-wrap gap-1.5" aria-label="Filter by state">
          {STATES.map((s) => (
            <li key={s}>
              <button
                type="button"
                aria-pressed={only === s}
                className={cn(
                  "flex items-center gap-1 rounded border px-1.5 py-0.5 text-[12px]",
                  only === s && "bg-accent",
                )}
                onClick={() => setOnly(only === s ? null : s)}
              >
                <Dot state={s} />
                {counts[s]} {STATE_WORD[s]}
              </button>
            </li>
          ))}
        </ul>
        <span className="text-muted-foreground ml-auto text-[12px]" role="status">
          {LIVE_WORD[live.state]}
          {picture.tier === "redacted" ? " · redacted" : ""}
          {live.reason && live.state !== "live" ? ` · ${live.reason}` : ""}
        </span>
      </header>
      {note ? <p className="text-muted-foreground text-[12px]">Agents: {note}</p> : null}
      {held.length > 0 ? (
        <p className="text-[12px]" role="status" data-blocked>
          {held.length} held up:{" "}
          {held
            .slice(0, 3)
            .map((b) => `${b.label} (${b.why}; ${b.who})`)
            .join(" · ")}
          {held.length > 3 ? ` · +${held.length - 3}` : ""}
        </p>
      ) : null}
      {empty && live.state === "live" ? <p className="notice">Nothing is running for this account yet.</p> : null}
      <div className="grid min-h-0 flex-1 gap-3 lg:grid-cols-[1fr_18rem]">
        <div className="min-h-[320px] min-w-0 overflow-auto rounded-md border">
          {view === "orbit" ? (
            webgl ? (
              <Suspense fallback={<p className="text-muted-foreground p-3 text-sm">Loading the 3D view…</p>}>
                <OrbitScene planets={planets} picked={picked} onPick={setPicked} reduced={reduced} />
              </Suspense>
            ) : (
              <>
                <p className="text-muted-foreground p-2 text-[12px]">
                  This browser has no WebGL, so the orbit is drawn flat.
                </p>
                <OrbitFlat planets={planets} picked={picked} onPick={setPicked} />
              </>
            )
          ) : view === "room" ? (
            <ul className="grid gap-2 p-2 sm:grid-cols-2 xl:grid-cols-3">
              {tiles.map((t) => (
                <li key={t.id}>
                  <button
                    type="button"
                    className={cn("w-full rounded-md border p-2 text-left", picked === t.id && "bg-accent")}
                    onClick={() => setPicked(t.members[0] ?? null)}
                  >
                    <span className="flex items-center gap-1.5">
                      <Dot state={t.state} />
                      <span className="truncate font-medium">{t.name}</span>
                    </span>
                    <span className="text-muted-foreground block text-[12px]">{t.sub}</span>
                    {t.latest ? <span className="block truncate text-[12px]">{t.latest}</span> : null}
                    {t.blocked ? (
                      <span className="block text-[12px]">
                        Held up: {t.blocked.why}; {t.blocked.who} unblocks it
                      </span>
                    ) : null}
                  </button>
                </li>
              ))}
            </ul>
          ) : (
            <ul aria-label="Workload tree" className="p-2 text-sm">
              {rows.map((r) => (
                <li
                  key={r.id}
                  aria-current={picked === r.id}
                  style={{ paddingLeft: `${r.depth * 16}px` }}
                  className="flex items-center gap-1.5 py-0.5"
                >
                  {r.hasChildren ? (
                    <button
                      type="button"
                      aria-label={r.open ? "Fold" : "Unfold"}
                      className="text-muted-foreground w-4"
                      onClick={() =>
                        setCollapsed((c) =>
                          c.has(r.id) ? new Set([...c].filter((x) => x !== r.id)) : new Set(c).add(r.id),
                        )
                      }
                    >
                      {r.open ? "▾" : "▸"}
                    </button>
                  ) : (
                    <span className="w-4" />
                  )}
                  <Dot state={r.node.state} />
                  <button
                    type="button"
                    className={cn("min-w-0 truncate text-left", picked === r.id && "font-medium")}
                    onClick={() => setPicked(r.id)}
                  >
                    {r.node.label}
                  </button>
                  <span className="text-muted-foreground truncate text-[12px]">{r.node.sub}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
        <div className="flex flex-col gap-2">
          {picked && map.nodes.get(picked) ? (
            <NodePanel key={picked} map={map} id={picked} onClose={() => setPicked(null)} onDone={setToast} />
          ) : (
            <p className="text-muted-foreground text-[12px]">Pick a planet, a moon or a tile. Esc shows all.</p>
          )}
          {toast ? (
            <p className="text-[12px]" role="status">
              {toast}
            </p>
          ) : null}
          {reduced ? (
            <p className="text-muted-foreground text-[12px]">Reduced motion is on: the orbit stands still.</p>
          ) : null}
        </div>
      </div>
    </section>
  );
}

export type { WNode };
