// A control-room tile's shell (Sensors' sensors, Workload's agents and job groups): the name, a line
// under it, a pin that keeps it first, and whatever the page puts in it. Pins are kept per page.

import { Pin, PinOff } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "./kit.tsx";

/** The pinned ids kept under `key` in localStorage; none when nothing is kept or it cannot be read. */
export const loadPins = (key: string): string[] => {
  try {
    const v = JSON.parse(localStorage.getItem(key) ?? "[]");
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
};

export function TileShell({
  name,
  sub,
  pinned,
  onPin,
  children,
  className,
  testid,
  data,
}: {
  name: string;
  sub?: ReactNode;
  pinned: boolean;
  onPin: () => void;
  children: ReactNode;
  className?: string;
  testid?: string;
  data?: Record<string, string | number>;
}) {
  const attrs = Object.fromEntries(Object.entries(data ?? {}).map(([k, v]) => [`data-${k}`, String(v)]));
  return (
    <li
      className={cn(
        "bg-card flex min-w-0 flex-col rounded-xl border px-3.5 py-3 shadow-xs",
        pinned && "ring-foreground/15 ring-1",
        className,
      )}
      data-testid={testid}
      {...attrs}
    >
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <p className="truncate text-[13px] font-medium" title={name}>
            {name}
          </p>
          {sub && <p className="text-muted-foreground truncate text-[11.5px]">{sub}</p>}
        </div>
        <button
          type="button"
          aria-pressed={pinned}
          aria-label={pinned ? `Unpin ${name}` : `Pin ${name}`}
          title={pinned ? "Unpin" : "Pin to the top"}
          onClick={onPin}
          className={cn(
            "hover:bg-accent focus-visible:ring-ring/50 -mt-0.5 -mr-1 rounded-md p-1 outline-none focus-visible:ring-[3px]",
            pinned ? "text-foreground" : "text-muted-foreground",
          )}
        >
          {pinned ? <PinOff className="size-3.5" /> : <Pin className="size-3.5" />}
        </button>
      </div>
      {children}
    </li>
  );
}
