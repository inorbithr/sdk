// "Your sensors" (PRD 0117, PRD 0077): the sensors this person registered, each with a switch, driven by the
// sensors service. Off pauses it, on resumes it, revoke is permanent and asked twice, and what a sensor sent can be
// read and deleted. Consent is per person: only sensors whose owner is this person's sign-in are listed, and the
// service decides who may act. Local checks on a Mac are read by the Mac app only; this page does not read a machine.
import { X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { Sensor } from "../lib/sensors-core.ts";
import { isOn, mineOf, pauseAllTargets, sentText, stateWord, switchAction } from "../lib/sensors-consent.ts";
import { Button, cn } from "../components/kit.tsx";
import { type Done, changeSensor, deleteSent, readSent } from "./actions.ts";

export function Switch({ on, label, onChange, disabled }: { on: boolean; label: string; onChange: (v: boolean) => void; disabled?: boolean }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!on)}
      className={cn(
        "focus-visible:ring-ring/50 relative h-6 w-10 shrink-0 rounded-full outline-none transition-colors focus-visible:ring-[3px] disabled:opacity-50",
        on ? "bg-foreground" : "bg-muted border",
      )}
    >
      <span className={cn("bg-background absolute top-0.5 size-5 rounded-full shadow transition-all motion-reduce:transition-none", on ? "left-[18px]" : "left-0.5")} />
    </button>
  );
}

type Ask = "revoke" | "delete" | null;

function Row({ s, onDone }: { s: Sensor; onDone: (d: Done) => void }) {
  const [busy, setBusy] = useState(false);
  const [sure, setSure] = useState<Ask>(null);
  const [sent, setSent] = useState<string | null>(null);
  const revoked = s.state === "revoked";
  const run = async (f: () => Promise<Done>) => {
    setBusy(true);
    onDone(await f());
    setBusy(false);
  };
  return (
    <li className="py-3" data-testid="your-sensor" data-sensor={s.id}>
      <div className="flex items-center gap-4">
        <div className="min-w-0 flex-1">
          <p className="truncate text-[13.5px] font-medium" title={s.name}>{s.name}</p>
          <p className="text-muted-foreground text-[12.5px]">
            {stateWord(s)}
            {s.producer ? ` · ${s.producer}` : ""}
          </p>
        </div>
        <Switch
          on={isOn(s)}
          label={`Send ${s.name}`}
          disabled={busy || revoked}
          onChange={(v) => {
            const a = switchAction(s, v);
            if (a) void run(() => changeSensor(s.id, a));
          }}
        />
      </div>
      <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
        <Button size="sm" variant="ghost" className="h-7 px-2 text-[12px]" disabled={busy} onClick={async () => {
          const r = await readSent(s.id);
          setSent(r.state === "ok" ? sentText(r.data) : r.state === "error" ? `Could not read: ${r.reason}` : "You are signed out.");
        }}>
          What was sent
        </Button>
        {sure === "delete" ? (
          <>
            <Button size="sm" variant="destructive" className="h-7 px-2 text-[12px]" disabled={busy} onClick={() => { setSure(null); void run(() => deleteSent(s.id)); }}>Delete what it sent</Button>
            <Button size="sm" variant="ghost" className="h-7 px-2 text-[12px]" onClick={() => setSure(null)}>Keep it</Button>
          </>
        ) : (
          <Button size="sm" variant="ghost" className="h-7 px-2 text-[12px]" disabled={busy} onClick={() => setSure("delete")}>Delete what was sent</Button>
        )}
        {!revoked &&
          (sure === "revoke" ? (
            <>
              <Button size="sm" variant="destructive" className="h-7 px-2 text-[12px]" disabled={busy} onClick={() => { setSure(null); void run(() => changeSensor(s.id, "revoke")); }}>Revoke for good</Button>
              <Button size="sm" variant="ghost" className="h-7 px-2 text-[12px]" onClick={() => setSure(null)}>Keep it</Button>
            </>
          ) : (
            <Button size="sm" variant="ghost" className="h-7 px-2 text-[12px]" disabled={busy} onClick={() => setSure("revoke")}>Revoke</Button>
          ))}
      </div>
      {sent && <p role="status" className="text-muted-foreground mt-1 text-[12px]">{sent}</p>}
    </li>
  );
}

/** The sheet over the Sensors page. Esc, the close button, Done or a click outside closes it. */
export function YourSensors({ sensors, subject, onChanged, onClose }: { sensors: readonly Sensor[]; subject: string; onChanged: () => void; onClose: () => void }) {
  const box = useRef<HTMLDivElement>(null);
  const close = useRef(onClose);
  close.current = onClose;
  const [said, setSaid] = useState<Done | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    const back = document.activeElement as HTMLElement | null;
    box.current?.querySelector<HTMLElement>("[role=switch], button")?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        close.current();
        return;
      }
      if (e.key !== "Tab" || !box.current) return;
      const els = [...box.current.querySelectorAll<HTMLElement>("button:not([disabled]), [href], input, [tabindex]:not([tabindex='-1'])")];
      if (!els.length) return;
      const first = els[0];
      const last = els[els.length - 1];
      const inside = box.current.contains(document.activeElement);
      if (e.shiftKey && (document.activeElement === first || !inside)) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && (document.activeElement === last || !inside)) {
        e.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      back?.focus?.();
    };
  }, []);
  const mine = mineOf(sensors, subject);
  const running = pauseAllTargets(mine);
  const done = (d: Done) => {
    setSaid(d);
    onChanged();
  };
  const pauseAll = async () => {
    setBusy(true);
    const rs = await Promise.all(running.map((s) => changeSensor(s.id, "pause")));
    const failed = rs.filter((r) => !r.ok).length;
    done(failed ? { ok: false, text: `${rs.length - failed} paused, ${failed} could not be.` } : { ok: true, text: `Paused ${rs.length}.` });
    setBusy(false);
  };
  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center bg-black/40 p-4 sm:items-center" onMouseDown={(e) => e.target === e.currentTarget && onClose()} data-testid="your-sensors-backdrop">
      <div ref={box} role="dialog" aria-modal="true" aria-labelledby="your-sensors-title" className="bg-card text-card-foreground flex max-h-[85vh] w-full max-w-lg flex-col rounded-xl border shadow-lg" data-testid="your-sensors">
        <header className="flex items-start justify-between gap-4 px-5 pt-4">
          <div>
            <h2 id="your-sensors-title" className="text-[15px] font-semibold">Your sensors</h2>
            <p className="text-muted-foreground mt-1 text-[13px] text-pretty">
              The sensors you registered. Each one sends only while it is on. Turning one off pauses it; revoking stops it for good.
            </p>
          </div>
          <Button variant="ghost" size="sm" aria-label="Close" onClick={onClose} className="-mr-2 px-2">
            <X />
          </Button>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto px-5">
          {mine.length === 0 ? (
            <p className="text-muted-foreground py-6 text-[13px] text-pretty">You have not registered a sensor on this account. Built-in sensors, and other people's, are not yours to switch.</p>
          ) : (
            <ul className="divide-y">
              {mine.map((s) => (
                <Row key={s.id} s={s} onDone={done} />
              ))}
            </ul>
          )}
          <p className="text-muted-foreground border-t py-3 text-[12px] text-pretty">
            Checks on a Mac are read by the Mac app only, with their own switches; this page does not read your machine. Readings from <code>iohr doctor</code> will show here once they report to the sensors service.
          </p>
        </div>
        <footer className="flex flex-wrap items-center justify-between gap-3 border-t px-5 py-3">
          <p className={cn("text-[12.5px]", said && !said.ok ? "text-destructive" : "text-muted-foreground")} aria-live="polite">
            {said?.text ?? (running.length ? `${running.length} of yours ${running.length === 1 ? "is" : "are"} sending.` : "None of yours is sending.")}
          </p>
          <div className="flex gap-2">
            {running.length > 0 && (
              <Button variant="outline" size="sm" disabled={busy} onClick={() => void pauseAll()}>
                Pause all of mine
              </Button>
            )}
            <Button size="sm" onClick={onClose}>Done</Button>
          </div>
        </footer>
      </div>
    </div>
  );
}
