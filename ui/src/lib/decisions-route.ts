// Where the Decisions pages live: `/decisions` and the Mac app's route grammar after it (`/decisions/<space>/<doc>`,
// `/decisions/~all/kind:rfc`). The server answers index.html for any such path, so no server change is needed.
import { parseRoute, type Route } from "./decisions-views.ts";

export const DECISIONS = "/decisions";

/** The route for a pathname, or null when the path is not Decisions. */
export function decisionsRoute(pathname: string): Route | null {
  if (pathname !== DECISIONS && !pathname.startsWith(`${DECISIONS}/`)) return null;
  return parseRoute(decodeURIComponent(pathname.slice(DECISIONS.length + 1)).replace(/\/+$/, ""));
}

/** The path of a route's item (what follows `/decisions/`), each part encoded. */
export const decisionsPath = (...parts: string[]): string =>
  parts.length === 0 ? DECISIONS : `${DECISIONS}/${parts.map(encodeURIComponent).join("/")}`;
