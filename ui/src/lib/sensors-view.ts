// Which of the Sensors page's two views shows (the Mac's `orbit` and `control`): the address's `?view=` wins, else the
// one kept in this browser, else the orbit. Pure, so node tests it; the page stores the choice with `stored`.
import { stored } from "./stored.ts";

export type View = "orbit" | "control";
export const SENSORS = "/sensors";

export const asView = (v: unknown): View | null => (v === "orbit" || v === "control" ? v : null);

/** The route is the Sensors page: `/sensors` with or without a trailing slash. */
export const sensorsRoute = (pathname: string): boolean => pathname.replace(/\/$/, "") === SENSORS;

/** The view from the address's query, the kept one, then the orbit. */
export const viewFrom = (search: string, kept: unknown): View => asView(new URLSearchParams(search).get("view")) ?? asView(kept) ?? "orbit";

export const viewStore = stored<View>("inorbit.sensors.view", (o) => asView(o) ?? "orbit");

/** The address for a view, keeping nothing else: `/sensors?view=control` (the orbit is the bare path). */
export const viewHref = (v: View): string => (v === "orbit" ? SENSORS : `${SENSORS}?view=${v}`);
