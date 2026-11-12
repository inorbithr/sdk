import type { Panel } from "../lib/extensions.ts";
import { panelSrc } from "../lib/panels.ts";
import { PANEL_FRAME } from "./sandbox.ts";

/**
 * One panel. A running panel (one this serve started, so it has a `port`) is framed at its own loopback origin with
 * the sandbox in `PANEL_FRAME`; the page's `frame-src` allows exactly the ports the serve started. A panel that is
 * not running shows what it is and a Start button.
 */
export function PanelFrame({
  panel,
  busy,
  reason,
  onStart,
  onClose,
}: {
  panel: Panel;
  busy: boolean;
  reason: string | null;
  onStart: () => void;
  onClose: () => void;
}) {
  const src = panel.port === undefined ? null : panelSrc(panel.port, panel.entry);
  return (
    <section className="panel" aria-label={panel.title}>
      <header className="panel-head">
        <h2>{panel.title}</h2>
        <span className="panel-meta">
          {panel.name} {panel.version}
          {src ? ` on 127.0.0.1:${panel.port}` : ""}
        </span>
        <button type="button" className="panel-close" onClick={onClose} disabled={busy}>
          Close
        </button>
      </header>
      {src ? (
        <iframe className="panel-frame" title={panel.title} src={src} {...PANEL_FRAME} />
      ) : (
        <div className="panel-idle">
          <p>
            {panel.name} {panel.version} is installed and declares a panel. Starting it runs the extension's program on
            this machine with the privileges you confirmed when you installed it.
          </p>
          {reason ? (
            <p className="notice" role="alert">
              {reason}
            </p>
          ) : null}
          <button type="button" onClick={onStart} disabled={busy}>
            {busy ? "Starting…" : "Start"}
          </button>
        </div>
      )}
    </section>
  );
}
