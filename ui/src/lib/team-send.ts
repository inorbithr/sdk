// A stand-in for `apiSend` until feat/ui-live publishes it in `lib/api.ts` (board, Contracts: same name, same
// signature, same `Read<T>` answer). Teams imports `apiSend` from here and the import moves to `./api.ts` when live
// lands; this file is then deleted. It adds no route of its own: it only sends to `/api/proxy/v1/...`.

import { proxyPath, type Read } from "./api.ts";
import { startToken } from "./panels.ts";

export async function apiSend<T = unknown>(
  method: "POST" | "PATCH" | "PUT" | "DELETE",
  path: string,
  body?: unknown,
  opts: { query?: Record<string, string>; signal?: AbortSignal } = {},
  token = startToken(),
  fetcher: typeof fetch = fetch,
): Promise<Read<T>> {
  const url = proxyPath(path, opts.query ?? {});
  if (url === null) return { state: "error", status: 400, reason: "That is not a path the page may call." };
  if (token === null)
    return { state: "error", status: 0, reason: "This page has no start token: open it with `iohr ui open`." };
  let res: Response;
  try {
    res = await fetcher(url, {
      method,
      headers: { "x-iohr-token": token, ...(body === undefined ? {} : { "content-type": "application/json" }) },
      body: body === undefined ? undefined : JSON.stringify(body),
      cache: "no-store",
      signal: opts.signal,
    });
  } catch {
    return { state: "error", status: 0, reason: "The local `iohr ui serve` did not answer." };
  }
  if (res.status === 401) return { state: "signed-out" };
  const text = await res.text().catch(() => "");
  let json: unknown = {};
  try {
    json = text === "" ? {} : JSON.parse(text);
  } catch {
    json = {};
  }
  if (!res.ok) {
    const e = (json as { error?: unknown }).error;
    const m = typeof e === "string" ? e : (e as { message?: unknown } | undefined)?.message;
    return {
      state: "error",
      status: res.status,
      reason: typeof m === "string" && m !== "" ? m : `The API answered HTTP ${res.status}.`,
    };
  }
  return { state: "ok", data: json as T };
}
