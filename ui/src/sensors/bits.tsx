// Small pieces both views share: the health dot and word, and a sparkline.

import { cn } from "../components/kit.tsx";
import { HEALTH_WORD, type Health, type Reading, sparkPoints } from "../lib/sensors-core.ts";

const DOT: Record<Health, string> = {
  fine: "bg-success",
  degraded: "bg-warning",
  failing: "bg-destructive",
  silent: "bg-muted-foreground/50",
};
const TEXT: Record<Health, string> = {
  fine: "text-success",
  degraded: "text-warning",
  failing: "text-destructive",
  silent: "text-muted-foreground",
};

export function HealthDot({ health, className }: { health: Health; className?: string }) {
  return <span aria-hidden className={cn("inline-block size-2 shrink-0 rounded-full", DOT[health], className)} />;
}

export function HealthWord({ health, className }: { health: Health; className?: string }) {
  return (
    <span className={cn("inline-flex items-center gap-1.5 text-[12px] font-medium", TEXT[health], className)}>
      <HealthDot health={health} />
      {HEALTH_WORD[health]}
    </span>
  );
}

/** A line of the readings, or a plain note when there are fewer than two. */
/** A sparkline in the health's colour, or in `color` (a CSS colour) for a page with other tones. */
export function Spark({
  readings,
  health,
  color,
  w = 120,
  h = 28,
  className,
}: {
  readings: readonly Reading[];
  health?: Health;
  color?: string;
  w?: number;
  h?: number;
  className?: string;
}) {
  const pts = sparkPoints(readings, w, h);
  if (!pts)
    return (
      <span className={cn("text-muted-foreground flex items-center text-[11px]", className)} style={{ height: h }}>
        No chart yet
      </span>
    );
  return (
    <svg
      aria-hidden
      viewBox={`0 0 ${w} ${h}`}
      preserveAspectRatio="none"
      className={cn("block w-full", health && TEXT[health], className)}
      style={{ height: h, color: health ? undefined : color }}
    >
      <polyline
        points={pts}
        fill="none"
        stroke="currentColor"
        strokeWidth={1.5}
        vectorEffect="non-scaling-stroke"
        strokeLinejoin="round"
      />
    </svg>
  );
}
