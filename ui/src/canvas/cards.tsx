// The cards that are not plain boxes: a Decisions document, a diagram, an ask with its options, a pull request and a
// decision (PRD 0065). Slice 2 draws what the file holds, no live reads: a document's text is read in slice 3's layer.
import { FileText, GitPullRequest, HelpCircle, Network, Scale } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../frame/cn.ts";
import type { Item } from "./schema.ts";

const ICON = { document: FileText, diagram: Network, ask: HelpCircle, option: HelpCircle, pr: GitPullRequest, decision: Scale } as const;

export const isCard = (i: Item): i is Extract<Item, { type: keyof typeof ICON }> => i.type in ICON;

export function Card({ it, detail, ring }: { it: Extract<Item, { type: keyof typeof ICON }>; detail: "full" | "summary" | "block"; ring: string }) {
  const Icon = ICON[it.type];
  let sub: ReactNode = null;
  if (it.type === "document") sub = `${it.document.kind ?? "document"} ${it.document.display_number ?? ""}`.trim();
  else if (it.type === "diagram") sub = `${it.model.nodes.length} nodes, ${it.model.edges.length} edges${it.placeholder ? ", placeholder" : ""}`;
  else if (it.type === "ask") sub = it.text;
  else if (it.type === "option") sub = `${it.option}: ${it.means}`;
  else if (it.type === "pr") sub = `${it.repo} #${it.number}`;
  else if (it.type === "decision") sub = `${it.kind.toUpperCase()} ${it.number}`;
  return (
    <div
      className={cn("bg-card absolute flex flex-col gap-1 overflow-hidden rounded-md border p-2 text-[13px] shadow-sm", ring)}
      style={{ left: it.x, top: it.y, width: it.w, height: it.h }}
      data-card={it.type}
    >
      <div className="flex items-center gap-1.5 font-medium">
        <Icon className="text-muted-foreground size-4 shrink-0" />
        <span className="truncate">{it.title || it.type}</span>
      </div>
      {detail === "block" ? null : <div className="text-muted-foreground line-clamp-4 text-[12px]">{sub}</div>}
    </div>
  );
}
