import { FileText, LogIn, Radar } from "lucide-react";
import { useEffect, useState } from "react";
import { cn } from "./frame/cn.ts";
import type { SessionState } from "./lib/api.ts";
import {
  dayOf,
  type Event,
  FILTERS,
  type Filter,
  firstName,
  greeting,
  isFirstRun,
  type Line,
  lastCheck,
  ledeOf,
  type Ref,
  type Run,
  servicesOf,
  storyFilter,
  storyOf,
  stripLine,
  stripOf,
  watched,
} from "./lib/home-core.ts";
import { type HomeData, type Part, useHome, useRuns } from "./lib/home-data.ts";

// The welcome page (RFC 0217, slice 3): the Mac app's "Story" Home. Io tells the day as a few plain sentences, each
// one marked back to the event that proves it, newest first on a time rail. Everything on it is what the API answered
// for the signed-in person, or a line that says it is not built here yet. Nothing is filled in to look like data.

const card = "bg-card text-card-foreground rounded-2xl border shadow-xs";
const when = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });

export function Home({
  session,
  busy,
  onSignIn,
}: {
  session: SessionState | null;
  busy: boolean;
  onSignIn: () => void;
}) {
  if (session === null) return <p className="text-muted-foreground text-sm">Checking sign-in…</p>;
  if (session.state === "unreachable")
    return (
      <section className={cn(card, "mx-auto max-w-xl p-6")} role="alert" data-home-state="unreachable">
        <h1 className="text-lg font-semibold">This page cannot reach its local server</h1>
        <p className="text-muted-foreground mt-1 text-sm">{session.reason}</p>
      </section>
    );
  if (session.state === "signed-out")
    return (
      <section className={cn(card, "mx-auto max-w-xl p-6")} data-home-state="signed-out">
        <h1 className="text-lg font-semibold">Sign in to see your day</h1>
        <p className="text-muted-foreground mt-1 text-sm">
          Home shows what your account says: what waits for you, what is down and what changed. It is signed in the way
          `iohr login` is, and this page never holds the credential.
        </p>
        {session.login === "failed" ? (
          <p className="text-destructive mt-2 text-sm" role="alert">
            Sign-in failed{session.reason ? `: ${session.reason}` : "."}
          </p>
        ) : null}
        <button
          type="button"
          className="bg-primary text-primary-foreground mt-4 flex items-center gap-1.5 rounded-md px-3 py-1.5 text-[13px] disabled:opacity-50"
          disabled={busy || session.login === "waiting"}
          onClick={onSignIn}
        >
          <LogIn aria-hidden="true" className="size-3.5" />
          {session.login === "waiting" ? "Finish in your browser…" : "Sign in"}
        </button>
      </section>
    );
  return <Story account={session.account === "" ? null : session.account} />;
}

function Story({ account }: { account: string | null }) {
  const h = useHome(account);
  const [filter, setFilter] = useState<Filter>("all");
  const [hot, setHot] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 5_000);
    return () => window.clearInterval(id);
  }, []);

  const services = h.monitors.state === "ok" ? servicesOf(h.monitors.data) : [];
  const watching = watched(services);
  const down = watching.filter((s) => !s.up && !s.unknown);
  const unknown = watching.filter((s) => s.unknown);
  const reviews = h.reviews.state === "ok" ? h.reviews.data : [];
  const spaces = h.spaces.state === "ok" ? h.spaces.data : [];
  const events = storyOf({
    services: watching,
    incidents: h.incidents.state === "ok" ? h.incidents.data : [],
    reviews,
    spaces,
  });
  const lede = ledeOf(events, { services: watching.length, down: down.length, unknown: unknown.length });
  const refs = lede.flatMap((l) => (l.ref && "event" in l.ref ? [l.ref.event] : []));
  const shown = storyFilter(events, filter);
  const loading = [h.reviews, h.spaces, h.monitors, h.incidents].some((p) => p.state === "loading");
  const pulse = lastCheck(
    watching.map((s) => s.lastRun),
    now,
  );
  const name = h.me.state === "ok" ? firstName(h.me.data) : "";
  const failed = problemsOf(h);
  const first =
    !loading &&
    failed.length === 0 &&
    isFirstRun({ decisions: reviews.length, services: watching.length, projects: spaces.length });

  const go = (r: Ref) => {
    const behavior = matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth";
    if ("filter" in r) {
      setFilter(r.filter);
      requestAnimationFrame(() => document.getElementById("story-rail")?.scrollIntoView({ behavior, block: "start" }));
      return;
    }
    setFilter("all");
    setHot(r.event);
    requestAnimationFrame(() =>
      document.getElementById(`ev-${r.event}`)?.scrollIntoView({ behavior, block: "center" }),
    );
    window.setTimeout(() => setHot((x) => (x === r.event ? null : x)), 2400);
  };

  let lastDay = "";
  return (
    <div className="@container mx-auto w-full max-w-6xl" data-home="story">
      <div className="grid gap-8 @3xl:grid-cols-[minmax(0,8fr)_minmax(0,4fr)]">
        <div className="min-w-0">
          <p className="text-muted-foreground flex items-center gap-1.5 text-[12.5px]" data-home-watching>
            <span className="size-2 rounded-full bg-amber-500" />
            {h.monitors.state === "ok"
              ? `Watching ${watching.length} ${watching.length === 1 ? "service" : "services"}${pulse === "" ? "" : ` · newest check ${pulse}`}${h.moreMonitors ? " · first 200 shown" : ""}`
              : h.monitors.state === "loading"
                ? "Reading your services…"
                : "Services could not be read"}
          </p>
          <p className="text-muted-foreground mt-6 text-[15px]">
            {greeting(name, new Date().getHours())}. Today, with evidence:
          </p>
          <h1
            className="mt-2 text-[24px] leading-[1.25] font-semibold tracking-[-0.02em] break-words text-pretty @xl:text-[34px]"
            data-home-lede
          >
            {loading
              ? "Reading your day…"
              : lede.map((l) => (
                  <span key={l.text}>
                    <LedeLine
                      line={l}
                      n={"event" in (l.ref ?? {}) ? refs.indexOf((l.ref as { event: string }).event) + 1 : 0}
                      onHot={setHot}
                      onGo={go}
                    />{" "}
                  </span>
                ))}
          </h1>
          <p
            className="text-muted-foreground mt-4 flex max-w-2xl items-start gap-2 text-[12px] leading-relaxed"
            data-not-built="proof"
          >
            <span className="bg-muted mt-px shrink-0 rounded px-1.5 py-0.5 text-[10.5px] font-medium tracking-wide uppercase">
              Not built yet
            </span>
            <span>
              A step-by-step proof under each line, and Io answering why on its own. Each line below says whether a
              check saw it or the record says so.
            </span>
          </p>

          {failed.length > 0 && (
            <div className="border-destructive/40 mt-4 rounded-lg border p-3 text-[13px]" role="alert" data-home-errors>
              <p className="font-medium">Some of this could not be read, so it is left out:</p>
              <ul className="text-muted-foreground mt-1 list-disc pl-5">
                {failed.map((f) => (
                  <li key={f.what}>
                    {f.what}: {f.reason}
                  </li>
                ))}
              </ul>
              <button type="button" className="mt-2 underline underline-offset-2" onClick={h.again}>
                Try again
              </button>
            </div>
          )}

          {first && (
            <div className={cn(card, "mt-6 p-5")} data-home-first-run>
              <p className="text-[15px] font-semibold">Nothing to show yet</p>
              <p className="text-muted-foreground mt-1 text-[13px]">
                This account has no waiting decisions, no watched services and no projects. They appear here as soon as
                they exist.
              </p>
            </div>
          )}

          <fieldset id="story-rail" className="m-0 mt-8 flex min-w-0 scroll-mt-6 flex-wrap gap-1.5 border-0 p-0">
            <legend className="sr-only">What happened</legend>
            {FILTERS.map((f) => (
              <button
                key={f.id}
                type="button"
                aria-pressed={filter === f.id}
                data-filter={f.id}
                onClick={() => setFilter(f.id)}
                className={cn(
                  "flex items-center gap-1.5 rounded-full border px-3 py-1 text-[12.5px] transition-colors",
                  filter === f.id
                    ? "bg-foreground text-background border-transparent"
                    : "text-muted-foreground hover:text-foreground",
                )}
              >
                {f.label}
                <span className="tabular-nums opacity-60">{storyFilter(events, f.id).length}</span>
              </button>
            ))}
          </fieldset>

          <ol className="relative mt-6 ml-[52px] border-l pl-6 @xl:ml-[64px]" data-home-timeline>
            <li className="relative mb-6 flex items-center gap-2">
              <span className="text-muted-foreground absolute -left-[88px] w-14 text-right font-mono text-[11px] whitespace-nowrap @xl:-left-[92px]">
                Now
              </span>
              <span className="absolute -left-[31px] size-3 rounded-full border-2 border-amber-500 bg-amber-400" />
            </li>
            {shown.length === 0 && (
              <li className="text-muted-foreground py-4 text-[13px]">
                {loading ? "Reading…" : "Nothing happened in this view yet."}
              </li>
            )}
            {shown.map((e, i) => {
              const day = dayOf(e.at, now);
              const sep = day !== lastDay;
              lastDay = day;
              return (
                <li key={e.id} id={`ev-${e.id}`} className="relative pb-6" data-event={e.id}>
                  {sep && i > 0 && (
                    <p className="text-muted-foreground -ml-6 mb-4 border-t pt-3 pl-6 text-[11.5px] font-medium tracking-wide uppercase">
                      {day}
                    </p>
                  )}
                  <span className="text-muted-foreground absolute top-4 -left-[88px] w-14 text-right font-mono text-[11px] whitespace-nowrap tabular-nums @xl:-left-[92px]">
                    {e.timed ? when(e.at) : "No time"}
                  </span>
                  <span
                    className={cn(
                      "ring-background absolute top-[18px] -left-[30px] size-2.5 rounded-full ring-4",
                      e.kind === "problem" ? "bg-red-500" : e.kind === "decision" ? "bg-amber-500" : "bg-sky-500",
                    )}
                  />
                  <EventCard
                    e={e}
                    n={refs.indexOf(e.id)}
                    hot={
                      hot === e.id ||
                      (hot === "f:problems" && e.kind === "problem") ||
                      (hot === "f:decisions" && e.kind === "decision")
                    }
                    account={account}
                  />
                </li>
              );
            })}
          </ol>
        </div>

        <aside className="flex flex-col gap-5 @3xl:sticky @3xl:top-0 @3xl:self-start">
          <IoCard />
          <WorkingForYou />
        </aside>
      </div>
    </div>
  );
}

/** One sentence of the lede: plain text, or a button that goes to what proves it (its number is the event's footnote). */
function LedeLine({
  line,
  n,
  onHot,
  onGo,
}: {
  line: Line;
  n: number;
  onHot: (id: string | null) => void;
  onGo: (r: Ref) => void;
}) {
  const ref = line.ref;
  if (ref === undefined) return <>{line.text}</>;
  return (
    <button
      type="button"
      className="text-left underline decoration-amber-500/60 decoration-2 underline-offset-4 hover:decoration-amber-500"
      onMouseEnter={() => onHot("event" in ref ? ref.event : `f:${ref.filter}`)}
      onMouseLeave={() => onHot(null)}
      onClick={() => onGo(ref)}
    >
      {line.text}
      {"event" in ref && (
        <sup className="ml-0.5 align-super text-[0.42em] font-semibold text-amber-600 dark:text-amber-400">{n}</sup>
      )}
    </button>
  );
}

function problemsOf(h: HomeData): { what: string; reason: string }[] {
  const out: { what: string; reason: string }[] = [];
  const add = (what: string, p: Part<unknown>) => {
    if (p.state === "error") out.push({ what, reason: p.reason });
    if (p.state === "signed-out") out.push({ what, reason: "the sign-in has ended; sign in again" });
  };
  add("Waiting decisions", h.reviews);
  add("Projects", h.spaces);
  add("Services", h.monitors);
  add("Problems", h.incidents);
  return out;
}

function EventCard({ e, n, hot, account }: { e: Event; n: number; hot: boolean; account: string | null }) {
  const runs = useRuns(account, e.monitor);
  // "Seen by a check" only once the checks have loaded and are there; until then it is the record's word.
  const check = e.source === "check" && Array.isArray(runs) && runs.length > 0;
  const Icon = check ? Radar : FileText;
  return (
    <article className={cn(card, "p-4 transition-shadow @xl:p-5", hot && "ring-2 ring-amber-400/70")}>
      <div className="flex items-start gap-3">
        {n >= 0 && (
          <span className="mt-0.5 grid size-5 shrink-0 place-items-center rounded-full bg-amber-500 text-[11px] font-bold text-black">
            {n + 1}
          </span>
        )}
        <p className="min-w-0 flex-1 text-[15.5px] leading-snug font-semibold text-pretty">{e.claim}</p>
        <span
          className={cn(
            "flex shrink-0 items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-medium",
            check ? "bg-success/15 text-success" : "bg-muted text-muted-foreground",
          )}
          data-source={check ? "check" : "record"}
        >
          <Icon className="size-3" aria-hidden="true" />
          <span className="@max-[420px]:hidden">{check ? "Seen by a check" : "From the record"}</span>
        </span>
      </div>
      {e.detail !== "" && <p className="text-muted-foreground mt-1 text-[13px]">{e.detail}</p>}
      {e.monitor !== undefined && <CheckEvidence runs={runs} />}
    </article>
  );
}

function CheckEvidence({ runs }: { runs: Run[] | "failed" | null }) {
  return (
    <div className="bg-muted/40 mt-3 rounded-xl p-3" data-run-strip>
      <p className="text-muted-foreground mb-2 text-[11px] font-medium tracking-wide uppercase">How we know</p>
      {runs === "failed" ? (
        <p className="text-muted-foreground text-[12px]">The checks could not be read.</p>
      ) : runs === null ? (
        <div className="bg-muted h-5 w-40 animate-pulse rounded motion-reduce:animate-none" />
      ) : (
        <Strip runs={stripOf(runs)} />
      )}
    </div>
  );
}

function Strip({ runs }: { runs: Run[] }) {
  return (
    <div>
      <div className="flex items-end gap-[3px]" aria-hidden="true">
        {runs.map((r, i) => (
          <span
            // biome-ignore lint/suspicious/noArrayIndexKey: two runs may share a second
            key={`${r.at}${i}`}
            title={`${new Date(r.at).toLocaleTimeString()} · ${r.ok ? "passed" : "failed"}${r.status_code ? ` · ${r.status_code}` : ""}`}
            className={cn("w-[6px] rounded-[2px]", r.ok ? "bg-success/80 h-3.5" : "bg-destructive h-5")}
          />
        ))}
      </div>
      <p className="text-muted-foreground mt-1.5 text-[12px]">{stripLine(runs)}</p>
    </div>
  );
}

/** Io is the Mac app's chat and `iohr ask`; this page has no chat yet and says so. */
function IoCard() {
  return (
    <div className={cn(card, "p-4")} data-not-built="io">
      <div className="flex items-center gap-2">
        <span className="bg-muted grid size-9 place-items-center rounded-full text-[15px] font-semibold">Io</span>
        <span className="bg-muted rounded px-1.5 py-0.5 text-[10.5px] font-medium tracking-wide uppercase">
          Not built yet
        </span>
      </div>
      <p className="text-muted-foreground mt-3 text-[13px] leading-relaxed">
        Asking Io is not in this page yet. It works in the Mac app and with <code>iohr ask</code> in a terminal.
      </p>
    </div>
  );
}

/** The Roles live on the machine and are read by the Mac app and `iohr`; no API lists them for this page. */
function WorkingForYou() {
  return (
    <section data-not-built="roles" aria-labelledby="working-for-you">
      <div className="flex items-center gap-2">
        <h2 id="working-for-you" className="text-[15px] font-semibold">
          Working for you
        </h2>
        <span className="bg-muted rounded px-1.5 py-0.5 text-[10.5px] font-medium tracking-wide uppercase">
          Not built yet
        </span>
      </div>
      <p className="text-muted-foreground mt-1 text-[13px] leading-relaxed">
        Your Roles are not listed here yet: nothing this page can ask returns them. The Mac app lists them.
      </p>
    </section>
  );
}
