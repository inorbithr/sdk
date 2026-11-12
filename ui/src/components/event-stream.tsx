// The live event stream beside a control room (Sensors, Workload): newest first, filterable by
// group (a source, an agent) and tone (a health, a state). One column on a narrow window.

import { type ReactNode, useState } from "react";
import { filterStream, type StreamEvent } from "../lib/event-stream-core.ts";
import { cn } from "./kit.tsx";

const time = (at: number) =>
  new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });

export function EventStream<T extends string>({
  events,
  groups,
  tones,
  word,
  dot,
  groupLabel,
  lede,
  empty,
  testid,
}: {
  /** Newest first. */
  events: readonly StreamEvent<T>[];
  /** What the group filter offers, by id. */
  groups: readonly { id: string; name: string }[];
  /** The tones to filter by, in the order that matters. */
  tones: readonly T[];
  word: Record<T, string>;
  dot: (tone: T, className?: string) => ReactNode;
  /** The group filter's first option: "Every source", "Every agent". */
  groupLabel: string;
  lede: string;
  /** What the stream says with no event at all. */
  empty: string;
  testid?: string;
}) {
  const [group, setGroup] = useState<string>("");
  const [picked, setPicked] = useState<T[]>([]);
  const shown = filterStream(events, group || null, picked);
  const names = new Map(groups.map((g) => [g.id, g.name]));
  return (
    <aside
      aria-label="Event stream"
      className="bg-card flex min-h-0 flex-col rounded-xl border shadow-xs"
      data-testid={testid}
      data-shown={shown.length}
    >
      <header className="border-b px-4 pt-3 pb-2.5">
        <h2 className="text-sm font-medium">Event stream</h2>
        <p className="text-muted-foreground text-[12px]">{lede}</p>
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          <select
            aria-label="Filter by group"
            value={group}
            onChange={(e) => setGroup(e.target.value)}
            className="bg-background focus-visible:ring-ring/50 h-7 max-w-full rounded-md border px-2 text-[12px] outline-none focus-visible:ring-[3px]"
          >
            <option value="">{groupLabel}</option>
            {groups.map((g) => (
              <option key={g.id} value={g.id}>
                {g.name}
              </option>
            ))}
          </select>
          {tones.map((t) => {
            const on = picked.includes(t);
            return (
              <button
                key={t}
                type="button"
                aria-pressed={on}
                onClick={() => setPicked(on ? picked.filter((x) => x !== t) : [...picked, t])}
                data-testid="stream-tone"
                data-tone={t}
                className={cn(
                  "focus-visible:ring-ring/50 inline-flex h-7 items-center gap-1.5 rounded-md border px-2 text-[12px] outline-none focus-visible:ring-[3px]",
                  on ? "bg-foreground text-background border-transparent" : "hover:bg-accent text-muted-foreground",
                )}
              >
                {dot(t)}
                {word[t]}
              </button>
            );
          })}
        </div>
      </header>
      <ol className="min-h-0 flex-1 overflow-y-auto px-4 py-1" aria-live="polite" aria-relevant="additions">
        {shown.length === 0 && (
          <li className="text-muted-foreground py-3 text-[12.5px] text-pretty">
            {events.length ? "Nothing matches these filters." : empty}
          </li>
        )}
        {shown.map((e) => (
          <li
            key={e.id}
            className="flex items-start gap-2.5 border-b py-2 last:border-b-0"
            data-testid="stream-event"
            data-tone={e.tone}
          >
            {dot(e.tone, "mt-1.5")}
            <div className="min-w-0 flex-1">
              <p className={cn("text-[12.5px] break-words", e.strong && "font-medium")}>{e.text}</p>
              <p className="text-muted-foreground font-mono text-[10.5px]">
                {time(e.at)} · {names.get(e.group) ?? e.group}
              </p>
            </div>
          </li>
        ))}
      </ol>
    </aside>
  );
}
