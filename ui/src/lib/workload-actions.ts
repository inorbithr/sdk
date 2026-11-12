// What a person can do to a workload node from the page, as API writes. Each goes through apiSend, so it
// runs with the signed-in person's own rights: the server may refuse, and the page shows its answer.
// The Mac app's stop, restart and open-session have no web source (they act on local processes).
import type { WNode } from "./workload-map.ts";

export type WorkAction = {
  id: "cancel" | "release" | "withdraw";
  label: string;
  method: "POST";
  path: string;
  confirm: string;
};

const enc = encodeURIComponent;

/** The actions a node offers, for its kind and state. Empty when none applies (a finished job, an agent). */
export function actionsFor(n: WNode): WorkAction[] {
  if (n.state === "done") return [];
  if (n.kind === "job") {
    return [
      {
        id: "release",
        label: "Release",
        method: "POST",
        path: `/v1/coordinator/jobs/${enc(n.id)}/release`,
        confirm: `Release ${n.label}? It goes back to the queue.`,
      },
      {
        id: "cancel",
        label: "Cancel",
        method: "POST",
        path: `/v1/coordinator/jobs/${enc(n.id)}/cancel`,
        confirm: `Cancel ${n.label}? Its work stops.`,
      },
    ];
  }
  if (n.kind === "ask")
    return [
      {
        id: "withdraw",
        label: "Withdraw",
        method: "POST",
        path: `/v1/coordinator/asks/${enc(n.id)}/withdraw`,
        confirm: `Withdraw ${n.label}? The planner drops it.`,
      },
    ];
  return [];
}
