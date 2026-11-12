// Decisions reads (RFC 0217): the REST shapes of `/v1/decisions/...` (proto iohr/decisions/v1) read through `apiGet`,
// so the page holds no credential. Each read is a `Read<T>`: ok, signed-out or an error with the API's own reason.
import { apiGet, type Read } from "./api.ts";
import type { ListDoc } from "./decisions-views.ts";
import type { Via } from "./io-via.ts";

export type Space = {
  space_id: string;
  name: string;
  slug?: string;
  description?: string;
  documents?: number;
  open?: number;
  awaiting_review?: number;
  parent_id?: string;
  kind?: string;
  department?: string;
  depth?: number;
  children?: number;
  updated_at?: string;
};

export type Doc = ListDoc & {
  via?: Via | null;
  review_by?: string;
  current_version?: number;
  parent_id?: string;
  superseded_by?: string;
  supersedes_id?: string;
  access?: string;
  headline?: string;
};

export type DocResponse = {
  document: Doc;
  version: number;
  text: string;
  author?: string;
  via?: Via | null;
  message?: string;
  saved_at?: string;
  parent?: Doc | null;
  children?: Doc[];
  mentions?: Doc[];
  mentioned_in?: Doc[];
  implements?: Doc[];
  implemented_by?: Doc[];
  supersedes?: Doc | null;
  successors?: Doc[];
  findings?: { line?: number | null; rule: string; message: string }[];
};

const enc = encodeURIComponent;
export const spacePath = (space: string) => `/v1/decisions/spaces/${enc(space)}`;
export const docPath = (space: string, doc: string) => `${spacePath(space)}/documents/${enc(doc)}`;

const pick = <T, K extends string>(r: Read<Record<string, unknown>>, key: K): Read<T[]> =>
  r.state === "ok" ? { state: "ok", data: ((r.data[key] as T[] | undefined) ?? []) as T[] } : r;

/** Every space of the account, departments and sub-spaces included. */
export async function readTree(account: string, fetcher?: typeof fetch): Promise<Read<Space[]>> {
  return pick<Space, "spaces">(
    await apiGet<Record<string, unknown>>("/v1/decisions/tree", { account_id: account }, undefined, fetcher),
    "spaces",
  );
}

/** One space's documents, every page up to `max`. */
export async function readDocuments(space: string, max = 1000, fetcher?: typeof fetch): Promise<Read<Doc[]>> {
  let token = "";
  let out: Doc[] = [];
  do {
    const r = await apiGet<{ documents?: Doc[]; next_page_token?: string }>(
      `${spacePath(space)}/documents`,
      { page_size: "200", ...(token === "" ? {} : { page_token: token }) },
      undefined,
      fetcher,
    );
    if (r.state !== "ok") return r;
    out = [...out, ...(r.data.documents ?? [])];
    token = r.data.next_page_token ?? "";
  } while (token !== "" && out.length < max);
  return { state: "ok", data: out };
}

/** One document with its text at `version` (0 is the current one). */
export const readDocument = (space: string, doc: string, version = 0, fetcher?: typeof fetch) =>
  apiGet<DocResponse>(docPath(space, doc), version > 0 ? { version: String(version) } : {}, undefined, fetcher);
