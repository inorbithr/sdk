// What the browser can do for the 3D view: WebGL, and whether the person asked for less motion.
import { useEffect, useState } from "react";

/** True when a WebGL context can be made. A page with no WebGL gets the flat view instead. */
export function hasWebGL(doc: Pick<Document, "createElement"> = document): boolean {
  try {
    const c = doc.createElement("canvas") as HTMLCanvasElement;
    return !!(c.getContext("webgl2") ?? c.getContext("webgl"));
  } catch {
    return false;
  }
}

const QUERY = "(prefers-reduced-motion: reduce)";

/** True while the person's system asks for reduced motion; follows a change while the page is open. */
export function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState(() => window.matchMedia?.(QUERY).matches ?? false);
  useEffect(() => {
    const m = window.matchMedia?.(QUERY);
    if (!m) return;
    const on = () => setReduced(m.matches);
    m.addEventListener("change", on);
    return () => m.removeEventListener("change", on);
  }, []);
  return reduced;
}
