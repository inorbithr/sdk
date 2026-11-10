import type { ReactNode } from "react";
import { cn } from "../frame/cn.ts";
import type { LiveState } from "../lib/live-shim.ts";

export const card = "bg-card text-card-foreground rounded-2xl border shadow-xs";
export const button =
  "border-input hover:bg-accent inline-flex h-8 items-center rounded-md border px-3 text-[13px] disabled:opacity-50";
export const field = "border-input bg-background w-full rounded-md border px-2 py-1 text-[13px]";

const LIVE_WORD: Record<LiveState, string> = {
  connecting: "Connecting",
  live: "Live",
  reconnecting: "Reconnecting",
  off: "Offline",
};

/** The stream's state in a word, with why when it is not live (RFC 0217: SSE, a reconnect with a growing pause). */
export function LiveBadge({
  state,
  reason,
  what = "plan stream",
}: {
  state: LiveState;
  reason?: string | null;
  what?: string;
}) {
  return (
    <span
      role="status"
      data-live={state}
      title={reason ? `${what}: ${reason}` : what}
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px]",
        state === "live" ? "text-foreground" : "text-muted-foreground",
      )}
    >
      <span
        className={cn(
          "size-1.5 rounded-full",
          state === "live" ? "bg-green-500" : state === "off" ? "bg-red-500" : "bg-amber-500",
        )}
      />
      {LIVE_WORD[state]}
    </span>
  );
}

export const Notice = ({ children }: { children: ReactNode }) => (
  <p className="notice" role="alert">
    {children}
  </p>
);

export const Chip = ({
  children,
  tone = "idle",
  title,
}: {
  children: ReactNode;
  tone?: "good" | "warn" | "bad" | "idle";
  title?: string;
}) => (
  <span
    title={title}
    className={cn(
      "inline-block rounded-full border px-2 py-0.5 text-[11px]",
      tone === "good" && "text-green-700 dark:text-green-400",
      tone === "warn" && "text-amber-700 dark:text-amber-400",
      tone === "bad" && "text-red-700 dark:text-red-400",
      tone === "idle" && "text-muted-foreground",
    )}
  >
    {children}
  </span>
);

/** Stub values are labelled stubs wherever they show. */
export const StubChip = () => <Chip title="A stub answer, not a measurement">stub</Chip>;
