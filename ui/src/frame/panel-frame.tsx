import type { Panel } from "../lib/extensions.ts";
import { PANEL_FRAME } from "./sandbox.ts";

/**
 * One panel. `src` is set only by the panel start path, which waits on the security review of RFC 0217 question 3
 * (one launch path through `ext/run.rs` with the full trust check). Until then no caller passes it, and the frame
 * shows a labelled stub instead.
 */
export function PanelFrame({ panel, src }: { panel: Panel; src?: string }) {
  if (src === undefined) {
    return (
      <section className="panel-stub" data-stub="true" aria-label={`${panel.title} (not started)`}>
        <p className="stub-badge">Stub</p>
        <h2>{panel.title}</h2>
        <p>
          {panel.name} {panel.version} is installed and declares a panel. Starting panels from the web UI is not built
          yet: it waits on the security review (RFC 0217).
        </p>
      </section>
    );
  }
  return <iframe className="panel-frame" title={panel.title} src={src} {...PANEL_FRAME} />;
}
