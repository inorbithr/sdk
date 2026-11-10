// The orbit view (PRD 0117 slice D): every source is a planet and its sensors orbit it. Colour is
// health, orbit speed is the signal rate, and a new signal draws a streak from the sensor to its
// planet. Clicking a planet zooms in to its sensors, live readings, a chart per sensor and its recent
// sessions. The drawing is the shared orbit canvas (components/orbit-canvas.tsx); this file maps
// sources onto it and owns the planet panel.

import { useEffect, useMemo, useRef } from "react";
import { ArrowLeft } from "lucide-react";
import { Card } from "../components/kit.tsx";
import { OrbitCanvas, type OrbitPlanet, type ToneSpec } from "../components/orbit-canvas.tsx";
import { useReducedMotion } from "../lib/use-reduced-motion.ts";
import { Button } from "../components/kit.tsx";
import { agoText, byHealth, HEALTH_TOKEN, HEALTH_WORD, orbitSpeed, rateText, valueText, type Health, type SessionPreview, type SignalEvent, type Source } from "../lib/sensors-core.ts";
import { HealthDot, HealthWord, Spark } from "./bits.tsx";
import { SessionList } from "./sessions.tsx";

const FALLBACK: Record<Health, string> = { fine: "#16a34a", degraded: "#d97706", failing: "#dc2626", silent: "#888" };
/** Health on the canvas: the page's tokens; a fine or silent name stays muted so the colour is the planet's alone. */
export const HEALTH_TONES: Record<Health, ToneSpec> = Object.fromEntries((Object.keys(HEALTH_WORD) as Health[]).map((h) => [h, { token: HEALTH_TOKEN[h], fallback: FALLBACK[h], word: HEALTH_WORD[h], quiet: h === "fine" || h === "silent" }])) as Record<Health, ToneSpec>;

/** A source as a planet: its sensors in orbit, each at its signal rate. */
export function planetOf(src: Source): OrbitPlanet<Health> {
  return {
    id: src.id,
    name: src.name,
    tone: src.health,
    say: `${src.name}: ${HEALTH_WORD[src.health].toLowerCase()}, ${src.sensors.length} sensor${src.sensors.length === 1 ? "" : "s"}. Open.`,
    moons: src.sensors.map((s) => ({ id: s.id, name: s.name, tone: s.health, speed: orbitSpeed(s.rate_per_min, s.health, false), say: `${HEALTH_WORD[s.health]}, ${valueText(s)}, ${rateText(s.rate_per_min)}` })),
  };
}

export function OrbitView({
  sources,
  sessions,
  fresh,
  focus,
  onFocus,
  now,
  onOpen,
}: {
  sources: readonly Source[];
  sessions: readonly SessionPreview[] | null;
  fresh: readonly SignalEvent[];
  focus: string | null;
  onFocus: (id: string | null) => void;
  now: number;
  /** Called with the sensors of a planet when it is opened, so the page can read their readings. */
  onOpen?: (sensorIds: readonly string[]) => void;
}) {
  const reduced = useReducedMotion();
  const planets = useMemo(() => [...sources].sort(byHealth).map(planetOf), [sources]);
  const signalled = useMemo(() => fresh.filter((e) => e.kind === "signal").map((e) => e.sensor_id), [fresh]);
  const focused = focus ? (sources.find((s) => s.id === focus) ?? null) : null;
  const ids = focused?.sensors.map((x) => x.id).join("\n") ?? "";
  // biome-ignore lint/correctness/useExhaustiveDependencies: the planet's sensor ids are the trigger
  useEffect(() => {
    if (ids) onOpen?.(ids.split("\n"));
  }, [ids]);
  return (
    <OrbitCanvas
      planets={planets}
      tones={HEALTH_TONES}
      focus={focus}
      onFocus={onFocus}
      fresh={signalled}
      listLabel="Sources and sensors, as a list"
      backLabel="All sources"
      legend={
        <>
          {(["fine", "degraded", "failing", "silent"] as Health[]).map((h) => (
            <span key={h} className="inline-flex items-center gap-1">
              <HealthDot health={h} />
              {HEALTH_WORD[h]}
            </span>
          ))}
          <span>{reduced ? "Reduced motion: colour shows health" : "Faster orbit, more signals"}</span>
        </>
      }
      panel={focused && <PlanetPanel source={focused} sessions={sessions} now={now} onClose={() => onFocus(null)} />}
    />
  );
}

/** Which sessions belong on a planet: the coordinator's runs, the asks queue's asks. */
export function sessionsFor(source: Source, sessions: readonly SessionPreview[] | null): readonly SessionPreview[] | null {
  if (!sessions) return null;
  if (source.kind === "coordinator") return sessions.filter((s) => s.kind === "session");
  if (source.kind === "asks") return sessions.filter((s) => s.kind === "ask");
  return null;
}

function PlanetPanel({ source, sessions, now, onClose }: { source: Source; sessions: readonly SessionPreview[] | null; now: number; onClose: () => void }) {
  const own = sessionsFor(source, sessions);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ref.current?.querySelector<HTMLElement>("[data-panel-title]")?.focus();
  }, [source.id]);
  return (
    <div ref={ref} className="sensors-panel-in min-w-0">
      <Card
        title={
          <span tabIndex={-1} data-panel-title className="outline-none">
            {source.name}
          </span>
        }
        description={source.detail}
        aside={<HealthWord health={source.health} />}
        bodyClass="pb-2"
      >
        <ul className="divide-y">
          {[...source.sensors].sort(byHealth).map((s) => (
            <li key={s.id} className="py-2.5">
              <div className="flex items-baseline gap-2">
                <HealthDot health={s.health} className="translate-y-[-1px]" />
                <p className="min-w-0 flex-1 truncate text-[13px] font-medium" title={s.name}>
                  {s.name}
                </p>
                <p className="shrink-0 text-[13px] tabular-nums">{s.value == null && !s.text ? <span className="text-muted-foreground">No reading</span> : valueText(s)}</p>
              </div>
              <Spark readings={s.readings} health={s.health} h={26} className="mt-1.5" />
              <p className="text-muted-foreground mt-1 text-[11.5px]">
                {HEALTH_WORD[s.health]} · {rateText(s.rate_per_min)} · last {agoText(s.last_seen, now)}
              </p>
            </li>
          ))}
        </ul>
      </Card>
      <Card className="mt-4" title="Recent sessions" bodyClass="pb-2">
        {own == null ? (
          <p className="text-muted-foreground pb-2 text-[12.5px]">{sessions == null ? "Looking for sessions…" : "Sessions run on the coordinator. This source has none of its own."}</p>
        ) : (
          <SessionList sessions={own.slice(0, 8)} now={now} empty="Nothing running here right now." />
        )}
      </Card>
      <Button variant="ghost" size="sm" className="mt-2" onClick={onClose}>
        <ArrowLeft /> Back to all sources <span className="text-muted-foreground font-mono text-[11px]">esc</span>
      </Button>
    </div>
  );
}
