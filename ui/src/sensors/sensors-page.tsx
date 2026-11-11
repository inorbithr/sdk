// Sensors (RFC 0217, PRD 0117): every source the account's agents report and what each sensor says, live, in two
// views. The orbit draws sources as planets with their sensors in orbit; the control room is a tile per sensor beside
// the live event stream. "Your sensors" is the consent sheet for the person's own. What streams and what polls is said
// on the page: readings and changes arrive on the signal stream, sessions on the coordinator's picture stream.
import { LayoutGrid, Orbit, ShieldCheck } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Button, cn, Loading, Problems, Tag } from "../components/kit.tsx";
import type { SessionState } from "../lib/api.ts";
import { mineOf } from "../lib/sensors-consent.ts";
import { HEALTH_WORD, HEALTHS, type Health, type Source } from "../lib/sensors-core.ts";
import { type View, viewFrom, viewHref, viewStore } from "../lib/sensors-view.ts";
import { HealthDot } from "./bits.tsx";
import { ControlRoom } from "./control-room.tsx";
import { OrbitView } from "./orbit-view.tsx";
import { SAFETY_POLL_MS, useSensorsData } from "./use-sensors.ts";
import { YourSensors } from "./your-sensors.tsx";
import "./sensors.css";

const CONN_WORD = { connecting: "Connecting", live: "Live", reconnecting: "Reconnecting", offline: "Offline" } as const;

function ViewSwitch({ view, onView }: { view: View; onView: (v: View) => void }) {
  const opt = (v: View, label: string, Icon: typeof Orbit) => (
    <button
      type="button"
      aria-pressed={view === v}
      onClick={() => onView(v)}
      className={cn(
        "focus-visible:ring-ring/50 inline-flex h-7 items-center gap-1.5 rounded-[5px] px-2.5 text-[12.5px] font-medium outline-none focus-visible:ring-[3px]",
        view === v ? "bg-background text-foreground shadow-xs" : "text-muted-foreground hover:text-foreground",
      )}
    >
      <Icon className="size-3.5" />
      {label}
    </button>
  );
  return (
    <fieldset className="bg-muted m-0 inline-flex rounded-md border-0 p-0.5">
      <legend className="sr-only">View</legend>
      {opt("orbit", "Orbit", Orbit)}
      {opt("control", "Control room", LayoutGrid)}
    </fieldset>
  );
}

export function counts(sources: readonly Source[]): { health: Health; n: number }[] {
  const n: Record<Health, number> = { fine: 0, degraded: 0, failing: 0, silent: 0 };
  for (const s of sources) for (const x of s.sensors) n[x.health] += 1;
  return HEALTHS.map((health) => ({ health, n: n[health] }));
}

export function SensorsPage({ session, onSignIn }: { session: SessionState | null; onSignIn: () => void }) {
  const account = session?.state === "signed-in" ? session.account : "";
  const subject = session?.state === "signed-in" ? session.subject : "";
  const kept = viewStore.use();
  const [view, setView] = useState<View>(() => viewFrom(window.location.search, kept));
  const [focus, setFocus] = useState<string | null>(null);
  const [sheet, setSheet] = useState(false);
  const data = useSensorsData(account, subject);
  const sources = useMemo(() => data.snap?.sources ?? [], [data.snap]);
  const pick = (v: View) => {
    setView(v);
    viewStore.update(() => v);
    window.history.replaceState(null, "", viewHref(v));
  };
  // A planet that went away (its sensors revoked) closes.
  useEffect(() => {
    if (focus && data.snap && !sources.some((s) => s.id === focus)) setFocus(null);
  }, [focus, data.snap, sources]);

  // The control room draws a sparkline per tile: read the first few dozen sensors' readings.
  const tileIds =
    view === "control"
      ? sources
          .flatMap((s) => s.sensors.map((x) => x.id))
          .slice(0, 40)
          .join("\n")
      : "";
  // biome-ignore lint/correctness/useExhaustiveDependencies: the ids are the trigger
  useEffect(() => {
    if (tileIds) data.open(tileIds.split("\n"));
  }, [tileIds]);

  if (session === null) return <Loading what="Asking who you are signed in as…" />;
  if (session.state !== "signed-in")
    return (
      <div className="@container/page mx-auto w-full max-w-6xl" data-sensors="signed-out">
        <h1 className="text-xl font-semibold tracking-tight">Sensors</h1>
        <p className="text-muted-foreground mt-1 text-[13.5px]">Sign in to see what your account's sensors report.</p>
        <Button className="mt-4" onClick={onSignIn}>
          Sign in
        </Button>
      </div>
    );

  const cs = counts(sources);
  const total = cs.reduce((a, c) => a + c.n, 0);
  const mine = mineOf(data.snap ? sources.flatMap((s) => s.sensors) : [], subject);
  const now = data.snap?.at ?? Date.now();
  const problems = [...(data.snap?.problems ?? []), ...(data.signedOut ? ["You are signed out: sign in again."] : [])];

  return (
    <div className="@container/page mx-auto w-full max-w-6xl" data-sensors="page" data-view={view}>
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="text-muted-foreground text-[11.5px] font-medium">Mission Control</p>
          <h1 className="text-xl font-semibold tracking-tight">Sensors</h1>
          <p className="text-muted-foreground mt-1 max-w-prose text-[13.5px] text-pretty">
            Every source and what it reports, live. Colour is health, and in the orbit view a faster orbit means more
            signals.
          </p>
        </div>
        <Button size="sm" variant="outline" onClick={data.reload} disabled={data.loading}>
          {data.loading ? "Reading…" : "Reload"}
        </Button>
      </header>

      <div className="mt-5 flex flex-wrap items-center gap-x-3 gap-y-2">
        <ViewSwitch view={view} onView={pick} />
        <Button size="sm" variant="outline" onClick={() => setSheet(true)} data-testid="your-sensors-open">
          <ShieldCheck /> Your sensors{mine.length ? ` · ${mine.length}` : ""}
        </Button>
        {total > 0 && (
          <p className="text-muted-foreground flex flex-wrap items-center gap-x-3 gap-y-1 text-[12.5px]">
            <span className="sr-only">Sensors by health:</span>
            {cs
              .filter((c) => c.n > 0)
              .map((c) => (
                <span key={c.health} className="inline-flex items-center gap-1.5">
                  <HealthDot health={c.health} />
                  {c.n} {HEALTH_WORD[c.health].toLowerCase()}
                </span>
              ))}
          </p>
        )}
      </div>

      <p
        className="text-muted-foreground mt-3 flex flex-wrap items-center gap-2 text-[12.5px]"
        data-testid="sensors-conn"
      >
        <Tag className={cn(data.conn === "live" ? "border-success/40 text-success" : "border-warning/40 text-warning")}>
          {CONN_WORD[data.conn]}
        </Tag>
        <span>
          {data.conn === "live"
            ? "Readings arrive as they are sent. Sessions follow the coordinator's stream."
            : `The stream is ${CONN_WORD[data.conn].toLowerCase()}; the list is read again every ${SAFETY_POLL_MS / 1000} seconds meanwhile.`}
        </span>
      </p>
      {problems.length > 0 && (
        <div className="mt-3">
          <Problems errors={problems} />
        </div>
      )}

      <div className="mt-4">
        {!data.snap ? (
          <Loading
            what={
              account
                ? "Looking for sources and their sensors…"
                : "This sign-in names no account, so there are no sensors to read."
            }
          />
        ) : sources.length === 0 ? (
          <div
            className="text-muted-foreground rounded-xl border px-6 py-10 text-center text-[13.5px] text-pretty"
            data-testid="sensors-empty"
          >
            No sources yet. Sensors appear here when an agent on this account reports, and the built-in ones from
            monitors, incidents and the coordinator show as they have something to say.
          </div>
        ) : view === "orbit" ? (
          <OrbitView
            sources={sources}
            sessions={data.sessions}
            fresh={data.fresh}
            focus={focus}
            onFocus={setFocus}
            now={now}
            onOpen={data.open}
          />
        ) : (
          <ControlRoom sources={sources} events={data.events} now={now} onChanged={data.reload} />
        )}
      </div>

      {sheet && (
        <YourSensors
          sensors={sources.flatMap((s) => s.sensors)}
          subject={subject}
          onChanged={data.reload}
          onClose={() => setSheet(false)}
        />
      )}
    </div>
  );
}

export default SensorsPage;
