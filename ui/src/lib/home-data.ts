// What Home reads, and how (RFC 0217, slice 3): five plain GETs through the local server, as the same person
// `iohr login` signed in. Each answer stands alone: one failing leaves the others, and the page says which.
import { useCallback, useEffect, useRef, useState } from "react";
import { apiGet, type Read } from "./api.ts";
import type { Incident, Me, Monitor, Review, Run, Space } from "./home-core.ts";

const enc = encodeURIComponent;
const REFRESH_MS = 30_000;

/** `GET /v1/accounts/me`: who this is, their own account and their teams (the header, the switcher and Home read it). */
export const ME_READ = { path: "/v1/accounts/me", query: {} as Record<string, string> };

/** The API reads Home makes for an account (an org id). `null` account: nothing to ask, so nothing is asked. */
export function homeReads(account: string) {
  const org = `/v1/accounts/orgs/${enc(account)}`;
  return {
    me: ME_READ,
    reviews: { path: "/v1/decisions/reviews", query: { account_id: account, status: "waiting" } },
    spaces: { path: "/v1/decisions/tree", query: { account_id: account } },
    monitors: { path: `${org}/monitors`, query: { page_size: "200" } },
    incidents: { path: `${org}/incidents`, query: { page_size: "50" } },
  };
}

/** One monitor's latest checks. */
export const runsRead = (account: string, monitor: string) => ({
  path: `/v1/accounts/orgs/${enc(account)}/monitors/${enc(monitor)}/runs`,
  query: { page_size: "50" },
});

export type Part<T> =
  | { state: "loading" }
  | { state: "ok"; data: T }
  | { state: "signed-out" }
  | { state: "error"; reason: string };

export type HomeData = {
  me: Part<Me>;
  reviews: Part<Review[]>;
  spaces: Part<Space[]>;
  monitors: Part<Monitor[]>;
  /** More than one page of monitors exist; Home shows the first 200 and says so. */
  moreMonitors: boolean;
  incidents: Part<Incident[]>;
};

const LOADING: HomeData = {
  me: { state: "loading" },
  reviews: { state: "loading" },
  spaces: { state: "loading" },
  monitors: { state: "loading" },
  moreMonitors: false,
  incidents: { state: "loading" },
};

const part = <A, T>(r: Read<A>, pick: (a: A) => T): Part<T> =>
  r.state === "ok"
    ? { state: "ok", data: pick(r.data) }
    : r.state === "signed-out"
      ? { state: "signed-out" }
      : { state: "error", reason: r.reason };

type Wire = {
  reviews?: Review[];
  spaces?: Space[];
  monitors?: Monitor[];
  incidents?: Incident[];
  next_page_token?: string;
} & Me;

/** Reads everything Home shows, again every 30 seconds while the page is in front, and `again` reloads now. */
export function useHome(account: string | null): HomeData & { again: () => void } {
  const [data, setData] = useState<HomeData>(LOADING);
  const gen = useRef(0);

  const load = useCallback(async () => {
    if (account === null || account === "") return;
    const at = ++gen.current;
    const r = homeReads(account);
    const get = (x: { path: string; query: Record<string, string> }) => apiGet<Wire>(x.path, x.query);
    const [me, reviews, spaces, monitors, incidents] = await Promise.all([
      get(r.me),
      get(r.reviews),
      get(r.spaces),
      get(r.monitors),
      get(r.incidents),
    ]);
    if (at !== gen.current) return;
    setData({
      me: part(me, (a) => ({ name: a.name, email: a.email })),
      reviews: part(reviews, (a) => a.reviews ?? []),
      spaces: part(spaces, (a) => a.spaces ?? []),
      monitors: part(monitors, (a) => a.monitors ?? []),
      moreMonitors: monitors.state === "ok" && (monitors.data.next_page_token ?? "") !== "",
      incidents: part(incidents, (a) => a.incidents ?? []),
    });
  }, [account]);

  useEffect(() => {
    setData(LOADING);
    void load();
    const id = window.setInterval(() => {
      if (!document.hidden) void load();
    }, REFRESH_MS);
    return () => {
      gen.current++;
      window.clearInterval(id);
    };
  }, [load]);

  return { ...data, again: () => void load() };
}

/** One monitor's checks, newest 50: null while loading, "failed" when the read did not work. */
export function useRuns(account: string | null, monitor: string | undefined): Run[] | "failed" | null {
  const [runs, setRuns] = useState<Run[] | "failed" | null>(null);
  useEffect(() => {
    if (account === null || account === "" || monitor === undefined) return;
    let live = true;
    setRuns(null);
    const q = runsRead(account, monitor);
    void apiGet<{ runs?: Run[] }>(q.path, q.query).then((r) => {
      if (live) setRuns(r.state === "ok" ? (r.data.runs ?? []) : "failed");
    });
    return () => {
      live = false;
    };
  }, [account, monitor]);
  return runs;
}
