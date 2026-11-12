// The routes, and the one place a request is traced, logged and counted.

import { createServer } from "node:http";
import type { IncomingMessage, Server, ServerResponse } from "node:http";

import { Logger, Metrics, Span } from "./obs.ts";

/**
 * What the handlers share. `ready` is false until the server listens and again as soon as
 * shutdown starts, so a load balancer stops sending traffic before the process stops.
 */
export class Service {
  readonly log: Logger;
  readonly metrics = new Metrics();
  ready = false;

  constructor(log: Logger) {
    this.log = log;
  }
}

type Reply = { route: string; status: number; body: string; type: string };

const JSON_TYPE = "application/json";
const json = (route: string, status: number, body: unknown): Reply => ({
  route,
  status,
  body: JSON.stringify(body) + "\n",
  type: JSON_TYPE,
});

function respond(svc: Service, url: URL): Reply {
  switch (url.pathname) {
    case "/healthz":
      return json("/healthz", 200, { status: "ok" });
    case "/readyz":
      return svc.ready
        ? json("/readyz", 200, { status: "ready" })
        : json("/readyz", 503, { status: "not ready" });
    case "/metrics":
      return { route: "/metrics", status: 200, body: svc.metrics.render(), type: "text/plain; version=0.0.4" };
    case "/v1/hello": {
      // Your first route: replace it. The name comes from the query string, so it is bounded
      // and echoed back, never logged.
      const name = url.searchParams.get("name") || "world";
      if ([...name].length > 64) {
        return json("/v1/hello", 400, { error: "name is at most 64 characters of text" });
      }
      return json("/v1/hello", 200, { message: `Hello, ${name}` });
    }
    default:
      return json("unmatched", 404, { error: "not found" });
  }
}

export function makeServer(svc: Service): Server {
  return createServer((req: IncomingMessage, res: ServerResponse) => {
    const span = Span.start(req.headers["traceparent"] as string | undefined);
    const reply =
      req.method === "GET"
        ? respond(svc, new URL(req.url ?? "/", "http://localhost"))
        : json("unmatched", 405, { error: "method not allowed" });
    res.writeHead(reply.status, {
      "Content-Type": reply.type,
      "Content-Length": Buffer.byteLength(reply.body),
      traceparent: span.header(),
    });
    res.end(reply.body);
    const ms = span.millis();
    const method = req.method === "GET" ? "GET" : "OTHER";
    const common = { trace_id: span.traceId, route: reply.route, status: reply.status, duration_ms: ms };
    svc.log.info("span", {
      span_id: span.spanId,
      name: `${method} ${reply.route}`,
      ...common,
      ...(span.parentId ? { parent_span_id: span.parentId } : {}),
    });
    svc.log.info("request", { method, ...common });
    svc.metrics.inc(reply.route, reply.status);
  });
}
