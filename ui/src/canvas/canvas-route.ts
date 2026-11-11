// The canvas page's routes (PRD 0065): `/canvas` lists the canvases kept in this browser, `/canvas/<canvas_id>` opens
// one. Pure, so node tests it; app.tsx only asks whether the path is a canvas path.
export const CANVAS = "/canvas";

/** null when the path is not a canvas path; otherwise the canvas id (null for the list). */
export function canvasRoute(path: string): { id: string | null } | null {
  if (path === CANVAS || path === `${CANVAS}/`) return { id: null };
  if (!path.startsWith(`${CANVAS}/`)) return null;
  const rest = path.slice(CANVAS.length + 1).replace(/\/$/, "");
  if (!rest || rest.includes("/")) return null;
  try {
    return { id: decodeURIComponent(rest) };
  } catch {
    return null;
  }
}

export const canvasPath = (id: string | null) => (id ? `${CANVAS}/${encodeURIComponent(id)}` : CANVAS);
