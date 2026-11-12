// Decisions in the page (RFC 0217): the space tree, the overview, a space's documents, all documents with filters and
// the document reader, ported from the Mac app's Decisions page over the same REST API. Reads only in this slice;
// the views not yet here say so rather than show an empty shell.
import type { ReactNode } from "react";
import { useState } from "react";
import type { SessionState } from "../lib/api.ts";
import { type Doc, readDocument, readDocuments, readTree, type Space } from "../lib/decisions-api.ts";
import { statusText } from "../lib/decisions-core.ts";
import { decisionsPath, decisionsRoute } from "../lib/decisions-route.ts";
import {
  allRoute,
  docLabel,
  filterDocs,
  filtersOf,
  KIND_LABEL,
  KIND_ORDER,
  type Route,
  recentDocs,
  sortDocs,
  statusMix,
  typeCounts,
} from "../lib/decisions-views.ts";
import { MarkdownView } from "./markdown-view.tsx";
import { useRead } from "./use-read.ts";

// Poll intervals (the Decisions proto has no stream): the tree every 5 minutes, a document's text every minute.
const TREE_MS = 5 * 60_000;
const DOC_MS = 60_000;

const go = (path: string) => window.location.assign(path);
const link = (path: string) => ({
  href: path,
  onClick: (e: React.MouseEvent) => {
    if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return;
    e.preventDefault();
    go(path);
  },
});

function Note({ children, role = "status" }: { children: ReactNode; role?: "status" | "alert" }) {
  return (
    <p className="notice" role={role}>
      {children}
    </p>
  );
}

function Pill({ children }: { children: ReactNode }) {
  return <span className="bg-muted text-muted-foreground rounded-full px-2 py-0.5 text-[11px]">{children}</span>;
}

function DocRow({ d, space }: { d: Doc; space?: string }) {
  const sp = space ?? d.space_id ?? "";
  return (
    <li>
      <a
        {...link(decisionsPath(sp, d.document_id))}
        className="hover:bg-muted/60 flex flex-wrap items-baseline gap-x-3 gap-y-1 rounded-lg px-3 py-2"
      >
        <span className="text-muted-foreground w-20 shrink-0 text-[12px]">{docLabel(d)}</span>
        <span className="min-w-0 flex-1 font-medium">{d.title}</span>
        {d.status ? <Pill>{statusText(d.status)}</Pill> : null}
        {d.review_overdue ? <Pill>review overdue</Pill> : null}
      </a>
    </li>
  );
}

function Rail({ spaces, route }: { spaces: Space[]; route: Route }) {
  const here = "space" in route ? route.space : "";
  const top = spaces.filter((s) => !s.parent_id);
  const kids = (id: string) => spaces.filter((s) => s.parent_id === id);
  const item = (s: Space, depth: number): ReactNode => (
    <li key={s.space_id}>
      <a
        {...link(decisionsPath(s.space_id))}
        aria-current={here === s.space_id ? "page" : undefined}
        className={`hover:bg-muted/60 flex items-center justify-between gap-2 rounded-md py-1 pr-2 text-sm ${
          here === s.space_id ? "bg-muted font-medium" : ""
        }`}
        style={{ paddingLeft: 8 + depth * 12 }}
      >
        <span className="truncate">{s.name}</span>
        {s.documents ? <span className="text-muted-foreground text-[11px]">{s.documents}</span> : null}
      </a>
      {kids(s.space_id).length > 0 && <ul>{kids(s.space_id).map((k) => item(k, depth + 1))}</ul>}
    </li>
  );
  const view = (label: string, path: string, on: boolean) => (
    <li key={path}>
      <a
        {...link(path)}
        aria-current={on ? "page" : undefined}
        className={`hover:bg-muted/60 block rounded-md px-2 py-1 text-sm ${on ? "bg-muted font-medium" : ""}`}
      >
        {label}
      </a>
    </li>
  );
  return (
    <nav aria-label="Decisions" className="flex flex-col gap-3">
      <ul>
        {view("Overview", decisionsPath(), route.view === "overview")}
        {view("All documents", decisionsPath("~all"), route.view === "all")}
      </ul>
      <section aria-label="Spaces">
        <h2 className="text-muted-foreground px-2 pb-0.5 text-[11px] font-medium">Spaces</h2>
        <ul>{top.map((s) => item(s, 0))}</ul>
      </section>
    </nav>
  );
}

/** Every space's documents, read together (the account-wide list the Mac shows under All documents). */
function useAllDocs(spaces: Space[]) {
  const ids = spaces.map((s) => s.space_id).join(",");
  return useRead(
    async () => {
      const parts = await Promise.all(spaces.map((s) => readDocuments(s.space_id)));
      const data: Doc[] = [];
      for (const [i, p] of parts.entries()) {
        if (p.state !== "ok") return p;
        for (const d of p.data) data.push({ ...d, space_id: d.space_id ?? spaces[i]?.space_id });
      }
      return { state: "ok" as const, data };
    },
    `all:${ids}`,
    DOC_MS,
  );
}

function Overview({ spaces }: { spaces: Space[] }) {
  const docs = useAllDocs(spaces);
  if (docs === null) return <Note>Reading documents...</Note>;
  if (docs.state !== "ok") return <Note role="alert">{docs.state === "error" ? docs.reason : "Signed out."}</Note>;
  const list = docs.data;
  return (
    <div className="flex flex-col gap-6">
      <section aria-label="By type" className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        {typeCounts(list).map((t) => (
          <a
            key={t.kind}
            {...link(decisionsPath("~all", `kind:${t.kind}`))}
            className="bg-card rounded-2xl border p-4 shadow-xs"
          >
            <div className="text-2xl font-semibold">{t.count}</div>
            <div className="text-muted-foreground text-[12px]">{t.label}</div>
          </a>
        ))}
      </section>
      <section aria-label="By status" className="flex flex-wrap gap-2">
        {statusMix(list).map((s) => (
          <a key={s.status} {...link(decisionsPath("~all", `status:${s.status}`))}>
            <Pill>
              {statusText(s.status)} {s.count}
            </Pill>
          </a>
        ))}
      </section>
      <section aria-label="Recently changed">
        <h2 className="mb-1 text-base font-semibold">Recently changed</h2>
        <ul>
          {recentDocs(list).map((d) => (
            <DocRow key={d.document_id} d={d} />
          ))}
        </ul>
      </section>
    </div>
  );
}

function SpaceView({ space, spaces }: { space: string; spaces: Space[] }) {
  const info = spaces.find((s) => s.space_id === space);
  const docs = useRead(() => readDocuments(space), `space:${space}`, DOC_MS);
  return (
    <div className="flex flex-col gap-3">
      <header>
        <h1 className="text-xl font-semibold">{info?.name ?? space}</h1>
        {info?.description ? <p className="text-muted-foreground text-sm">{info.description}</p> : null}
      </header>
      {docs === null ? (
        <Note>Reading documents...</Note>
      ) : docs.state !== "ok" ? (
        <Note role="alert">{docs.state === "error" ? docs.reason : "Signed out."}</Note>
      ) : docs.data.length === 0 ? (
        <Note>This space has no documents yet.</Note>
      ) : (
        KIND_ORDER.map((k) => {
          const of = sortDocs(
            docs.data.filter((d) => d.kind === k),
            "number",
          );
          return of.length === 0 ? null : (
            <section key={k} aria-label={KIND_LABEL[k]}>
              <h2 className="text-muted-foreground px-3 text-[11px] font-medium">{KIND_LABEL[k]}s</h2>
              <ul>
                {of.map((d) => (
                  <DocRow key={d.document_id} d={d} space={space} />
                ))}
              </ul>
            </section>
          );
        })
      )}
    </div>
  );
}

function AllView({ route, spaces }: { route: Extract<Route, { view: "all" }>; spaces: Space[] }) {
  const docs = useAllDocs(spaces);
  const [query, setQuery] = useState("");
  const f = { ...filtersOf(route), query };
  const shown = docs?.state === "ok" ? sortDocs(filterDocs(docs.data, f), "updated") : [];
  return (
    <div className="flex flex-col gap-3">
      <header className="flex flex-wrap items-center gap-3">
        <h1 className="text-xl font-semibold">All documents</h1>
        <input
          type="search"
          aria-label="Filter documents"
          placeholder="Filter by title, number or author"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          className="bg-background min-w-0 flex-1 rounded-md border px-2 py-1 text-sm"
        />
      </header>
      <fieldset className="m-0 flex flex-wrap gap-2 border-0 p-0" aria-label="Filters">
        {(["kind", "status", "stage"] as const).map((k) =>
          route[k] ? (
            <a key={k} {...link(`/decisions/${allRoute({ ...route, [k]: undefined })}`)}>
              <Pill>
                {k}: {route[k]} (clear)
              </Pill>
            </a>
          ) : null,
        )}
      </fieldset>
      {docs === null ? (
        <Note>Reading documents...</Note>
      ) : docs.state !== "ok" ? (
        <Note role="alert">{docs.state === "error" ? docs.reason : "Signed out."}</Note>
      ) : shown.length === 0 ? (
        <Note>No document matches.</Note>
      ) : (
        <ul>
          {shown.map((d) => (
            <DocRow key={d.document_id} d={d} />
          ))}
        </ul>
      )}
    </div>
  );
}

function DocView({ space, doc }: { space: string; doc: string }) {
  const r = useRead(() => readDocument(space, doc), `doc:${space}:${doc}`, DOC_MS);
  if (r === null) return <Note>Reading the document...</Note>;
  if (r.state !== "ok")
    return <Note role="alert">{r.state === "error" ? r.reason : "Signed out: sign in from Settings."}</Note>;
  const { document: d } = r.data;
  return (
    <article className="flex flex-col gap-3">
      <header className="flex flex-col gap-1">
        <a {...link(decisionsPath(space))} className="text-muted-foreground text-[12px] underline">
          Back to the space
        </a>
        <h1 className="text-xl font-semibold">
          {docLabel(d)}: {d.title}
        </h1>
        <div className="flex flex-wrap items-center gap-2 text-[12px]">
          {d.status ? <Pill>{statusText(d.status)}</Pill> : null}
          <Pill>version {r.data.version}</Pill>
          {r.data.author ? <span className="text-muted-foreground">by {r.data.author}</span> : null}
          {r.data.saved_at ? (
            <span className="text-muted-foreground">saved {new Date(r.data.saved_at).toLocaleString()}</span>
          ) : null}
        </div>
      </header>
      {(r.data.findings?.length ?? 0) > 0 && (
        <Note>
          {r.data.findings?.length} check finding{r.data.findings?.length === 1 ? "" : "s"}:{" "}
          {r.data.findings?.map((x) => x.message).join("; ")}
        </Note>
      )}
      <div className="bg-card rounded-2xl border p-5 shadow-xs">
        <MarkdownView text={r.data.text} />
      </div>
    </article>
  );
}

const NEXT: Partial<Record<Route["view"], string>> = {
  waiting: "Waiting on you",
  mine: "Mine",
  queue: "The review queue",
  prs: "Linked pull requests",
  diagrams: "Diagrams",
  new: "Creating a document",
  "new-space": "Creating a space",
  settings: "Space settings",
  timeline: "A space's timeline",
};

export function Decisions({ session }: { session: SessionState | null }) {
  const route = decisionsRoute(window.location.pathname);
  const account = session?.state === "signed-in" ? session.account : "";
  const tree = useRead(
    () => (account === "" ? Promise.resolve({ state: "signed-out" as const }) : readTree(account)),
    `tree:${account}`,
    TREE_MS,
  );
  if (route === null) return null;
  if (session === null) return <Note>Reading who you are signed in as...</Note>;
  if (session.state !== "signed-in")
    return <Note>Decisions reads your account's documents: sign in from Settings, then come back.</Note>;
  if (tree === null) return <Note>Reading the spaces...</Note>;
  if (tree.state !== "ok")
    return <Note role="alert">{tree.state === "error" ? tree.reason : "Signed out: sign in from Settings."}</Note>;
  const spaces = tree.data;
  return (
    <div className="grid gap-6 md:grid-cols-[14rem_minmax(0,1fr)]" data-page="decisions">
      <aside>
        <Rail spaces={spaces} route={route} />
      </aside>
      <main className="min-w-0">
        {route.view === "overview" ? (
          <Overview spaces={spaces} />
        ) : route.view === "space" ? (
          <SpaceView space={route.space} spaces={spaces} />
        ) : route.view === "all" ? (
          <AllView route={route} spaces={spaces} />
        ) : route.view === "doc" ? (
          <DocView space={route.space} doc={route.doc} />
        ) : (
          <Note>{NEXT[route.view] ?? "This view"} is not in this page yet; the Mac app has it.</Note>
        )}
      </main>
    </div>
  );
}
