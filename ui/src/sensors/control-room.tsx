// The control room (PRD 0117 slice D): a tile per sensor with its value, a sparkline and its health,
// pinned tiles first, and the live event stream beside it, newest first, filterable by source and
// health. One column on a narrow window, the stream under the tiles. The tile shell and the stream
// are shared with Workload (components/tile.tsx, components/event-stream.tsx).

import { useMemo, useState } from "react";
import { EventStream } from "../components/event-stream.tsx";
import { TileShell } from "../components/tile.tsx";
import { Button } from "../components/kit.tsx";
import { changeSensor } from "./actions.ts";
import { stored } from "../lib/stored.ts";
import { agoText, HEALTH_WORD, HEALTHS, rateText, streamEvent, tileOrder, togglePin, valueText, type Sensor, type SignalEvent, type Source } from "../lib/sensors-core.ts";
import { HealthDot, HealthWord, Spark } from "./bits.tsx";

/** The pinned sensors, kept in this browser. */
export const pinsStore = stored<string[]>("inorbit.sensors.pins", (o) => (Array.isArray(o) ? o.filter((x): x is string => typeof x === "string") : []));

function Tile({ s, source, pinned, onPin, now, onChanged }: { s: Sensor; source: Source; pinned: boolean; onPin: () => void; now: number; onChanged?: () => void }) {
  const [said, setSaid] = useState<string | null>(null);
  const [sure, setSure] = useState(false);
  return (
    <TileShell name={s.name} sub={source.name} pinned={pinned} onPin={onPin}>
      <p className="mt-2 truncate text-[20px] leading-tight font-semibold tracking-[-0.02em] tabular-nums" title={valueText(s)}>
        {s.value == null && !s.text ? <span className="text-muted-foreground text-[15px] font-medium">No reading</span> : valueText(s)}
      </p>
      <Spark readings={s.readings} health={s.health} className="mt-2" />
      <div className="mt-2 flex items-center justify-between gap-2">
        <HealthWord health={s.health} />
        <span className="text-muted-foreground truncate text-[11px]">{s.last_seen ? agoText(s.last_seen, now) : rateText(s.rate_per_min)}</span>
      </div>
      {s.revocable && (
        <div className="mt-2 flex gap-1.5">
          {!sure ? (
            <Button size="sm" variant="ghost" className="h-7 px-2 text-[12px]" onClick={() => setSure(true)}>
              Revoke
            </Button>
          ) : (
            <>
              <Button
                size="sm"
                variant="destructive"
                className="h-7 px-2 text-[12px]"
                onClick={async () => {
                  setSure(false);
                  setSaid((await changeSensor(s.id, "revoke")).text);
                  onChanged?.();
                }}
              >
                Revoke for good
              </Button>
              <Button size="sm" variant="ghost" className="h-7 px-2 text-[12px]" onClick={() => setSure(false)}>
                Keep it
              </Button>
            </>
          )}
        </div>
      )}
      {said && (
        <p role="status" className="text-muted-foreground mt-1 text-[11.5px]">
          {said}
        </p>
      )}
    </TileShell>
  );
}

export function ControlRoom({ sources, events, now, onChanged }: { sources: readonly Source[]; events: readonly SignalEvent[]; now: number; onChanged?: () => void }) {
  const pins = pinsStore.use();
  const pin = (id: string) => pinsStore.update((p) => togglePin(p, id));
  const all = useMemo(() => sources.flatMap((s) => s.sensors), [sources]);
  const bySource = useMemo(() => new Map(sources.map((s) => [s.id, s])), [sources]);
  const tiles = tileOrder(all, pins);
  const stream = useMemo(() => events.map(streamEvent), [events]);
  return (
    <div className="grid grid-cols-1 gap-4 @3xl/page:grid-cols-[minmax(0,1fr)_300px]">
      <section aria-label="Sensors">
        {tiles.length === 0 ? (
          <p className="text-muted-foreground rounded-xl border px-4 py-6 text-center text-[13px]">No sensors to show yet.</p>
        ) : (
          <ul className="grid grid-cols-1 gap-3 @md/page:grid-cols-2 @2xl/page:grid-cols-3 @5xl/page:grid-cols-4">
            {tiles.map((s) => (
              <Tile key={s.id} s={s} source={bySource.get(s.source_id)!} pinned={pins.includes(s.id)} onPin={() => pin(s.id)} now={now} onChanged={onChanged} />
            ))}
          </ul>
        )}
      </section>
      <div className="@3xl/page:sticky @3xl/page:top-2 @3xl/page:max-h-[calc(100vh-7rem)] @3xl/page:self-start flex min-h-0">
        <EventStream
          events={stream}
          groups={sources.map((s) => ({ id: s.id, name: s.name }))}
          tones={HEALTHS}
          word={HEALTH_WORD}
          dot={(h, className) => <HealthDot health={h} className={className} />}
          groupLabel="Every source"
          lede="Newest first, from what this page has seen."
          empty="Nothing has changed since you opened this page. New signals and changes of health show here as they arrive."
        />
      </div>
    </div>
  );
}
