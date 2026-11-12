// Observability on the standard library alone: JSON logs, spans that follow the W3C
// traceparent header, and request counters in the Prometheus text format.
//
// Nothing here records a header, a query string or an environment value: a log line and a span
// name the route that matched, the status, the duration and the trace id.

import { randomBytes } from "node:crypto";

const TRACEPARENT = /^00-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$/;

export type Fields = Record<string, string | number | boolean>;

/** One JSON object per line to a sink (anything with `write`). */
export class Logger {
  #sink: { write(line: string): unknown };

  constructor(sink: { write(line: string): unknown }) {
    this.#sink = sink;
  }

  log(level: string, msg: string, fields: Fields = {}): void {
    this.#sink.write(JSON.stringify({ ts: new Date().toISOString(), level, msg, ...fields }) + "\n");
  }

  info(msg: string, fields: Fields = {}): void {
    this.log("INFO", msg, fields);
  }

  error(msg: string, fields: Fields = {}): void {
    this.log("ERROR", msg, fields);
  }
}

/** One request's trace: the id it joined (or started) and its own id. */
export class Span {
  readonly traceId: string;
  readonly spanId = randomBytes(8).toString("hex");
  readonly parentId: string | undefined;
  readonly #started = performance.now();

  private constructor(traceId: string, parentId?: string) {
    this.traceId = traceId;
    this.parentId = parentId;
  }

  /** Joins the trace of a valid traceparent header, or starts a new one. */
  static start(traceparent: string | undefined): Span {
    const m = TRACEPARENT.exec(traceparent ?? "");
    if (m && !/^0+$/.test(m[1]!) && !/^0+$/.test(m[2]!)) return new Span(m[1]!, m[2]!);
    return new Span(randomBytes(16).toString("hex"));
  }

  /** The traceparent to send back (and on to callers): this span is the parent. */
  header(): string {
    return `00-${this.traceId}-${this.spanId}-01`;
  }

  millis(): number {
    return Math.round((performance.now() - this.#started) * 1000) / 1000;
  }
}

/**
 * Request counts by route and status. Routes are matched patterns, never raw paths, so the
 * label set is bounded by the code and holds no user input.
 */
export class Metrics {
  #counts = new Map<string, number>();

  inc(route: string, status: number): void {
    const key = `route="${route}",status="${status}"`;
    this.#counts.set(key, (this.#counts.get(key) ?? 0) + 1);
  }

  render(): string {
    const lines = [
      "# HELP http_requests_total Requests served, by route and status.",
      "# TYPE http_requests_total counter",
    ];
    for (const key of [...this.#counts.keys()].sort()) {
      lines.push(`http_requests_total{${key}} ${this.#counts.get(key)}`);
    }
    return lines.join("\n") + "\n";
  }
}
