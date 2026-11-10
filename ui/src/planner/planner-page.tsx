import { openContextMenu } from "../frame/context-menu.tsx";
import { plannerMenu } from "../lib/planner-menus.ts";
import { MODE_LABEL, MODES, plannerHref, plannerRoute } from "../lib/planner-route.ts";
import { Overview } from "./overview.tsx";
import { DecisionsView, ExploreView, PreviewView } from "./plan-extra.tsx";
import { CanvasLink, PlanView, ProgressView } from "./plan-views.tsx";
import { useOverview, usePlan, useStages } from "./planner-data.ts";
import { LiveBadge, Notice } from "./ui.tsx";
import { WriteView } from "./write-view.tsx";

/** The Planner page (RFC 0217): `/planner` the overview, `/planner/<plan>[/<item>]?mode=` one plan in a mode. Live over the plan stream. */
export function PlannerPage() {
  const route = plannerRoute(window.location.pathname, window.location.search);
  const { live, state, reason } = useOverview();
  const planId = route?.planId ?? null;
  const { plan, listed, read, reload } = usePlan(planId, live.plans.get(planId ?? "")?.seq ?? 0);
  const st = useStages({ plan, listed });
  if (!route) return null;
  const mode = route.mode;
  const raw = read?.state === "ok" ? read.data : null;
  return (
    <section aria-label="Planner" className="flex flex-col gap-3">
      {/* biome-ignore lint/a11y/noStaticElementInteractions: the context menu is an extra, the modes are links below */}
      <header
        className="flex flex-wrap items-center gap-3"
        onContextMenu={(e) => {
          e.preventDefault();
          openContextMenu(
            { x: e.clientX, y: e.clientY },
            plannerMenu(route, {
              go: (h) => window.location.assign(h),
              copy: (t) => void navigator.clipboard?.writeText(`${window.location.origin}${t}`),
            }),
            { label: "Planner" },
          );
        }}
      >
        <h1 className="text-lg font-semibold">
          <a href={plannerHref({})}>Planner</a>
          {planId ? <span className="text-muted-foreground"> / {plan.title || planId}</span> : null}
        </h1>
        <LiveBadge state={state} reason={reason} />
      </header>
      {planId === null ? (
        <Overview plans={[...live.plans.values()]} ready={live.ready} />
      ) : (
        <>
          <nav aria-label="Planner modes" className="flex flex-wrap gap-1 text-[13px]">
            {MODES.map((m) => (
              <a
                key={m}
                href={plannerHref({ planId, itemId: route.itemId, mode: m })}
                aria-current={m === mode ? "page" : undefined}
                className={
                  m === mode
                    ? "bg-accent rounded-md px-2 py-1 font-medium"
                    : "text-muted-foreground rounded-md px-2 py-1"
                }
              >
                {MODE_LABEL[m]}
              </a>
            ))}
          </nav>
          {read?.state === "signed-out" ? <Notice>Sign in to read this plan.</Notice> : null}
          {read?.state === "error" ? <Notice>{read.reason}</Notice> : null}
          {st.reason ? <Notice>{st.reason}</Notice> : null}
          {mode === "plan" && <PlanView planId={planId} plan={plan} st={st} itemId={route.itemId} reload={reload} />}
          {mode === "progress" && <ProgressView plan={plan} st={st} />}
          {mode === "canvas" && <CanvasLink planId={planId} plan={plan} st={st} />}
          {mode === "preview" && <PreviewView planId={planId} plan={plan} st={st} />}
          {mode === "write" && (
            <WriteView planId={planId} plan={plan} raw={raw} itemId={route.itemId} reload={reload} />
          )}
          {mode === "decisions" && <DecisionsView plan={plan} />}
          {mode === "explore" && <ExploreView planId={planId} plan={plan} st={st} />}
        </>
      )}
    </section>
  );
}
