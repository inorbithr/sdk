/**
 * Streams (design.md section 7): the server-sent events parser and the multiplexed
 * socket (`/v1/ws`) a client's streams share when it opens them with `streams: "socket"`.
 * The client in `client.ts` opens both; nothing here is exported from the package
 * but the types.
 *
 * @module
 */

import {
  ApiError,
  CODES,
  ConnectionError,
  InOrbitError,
  RawResponse,
  TooLargeError,
} from "./errors.js";
import { backoffMs, sleep } from "./retry.js";

/** How a client opens its streams. */
export type StreamTransport = "sse" | "socket";

/** The largest event's data, and the longest line waiting for its end, 1 MiB. */
export const MAX_EVENT: number = 1024 * 1024;

/** Items one socket stream holds for a reader that has not taken them yet. */
export const SOCKET_QUEUE = 64;

/** One dispatched server-sent event: its name (`message` by default) and its data. */
export interface SseEvent {
  readonly event: string;
  readonly data: string;
}

/**
 * The WHATWG event-stream rules, fed text as it arrives: lines end with LF, CRLF or CR,
 * a line starting `:` is a comment, `data:` lines join with a newline (one leading space
 * dropped), `event:` names the event, a blank line dispatches, other fields are ignored.
 */
export class SseParser {
  #buffer = "";
  #data: string[] = [];
  #dataBytes = 0;
  #event = "";

  /** Feeds `text`; answers the events it completed. */
  push(text: string): SseEvent[] {
    this.#buffer += text;
    const out: SseEvent[] = [];
    for (;;) {
      const at = this.#buffer.search(/\r\n|\r|\n/);
      if (at < 0) {
        if (this.#buffer.length > MAX_EVENT) {
          throw new TooLargeError();
        }
        return out;
      }
      // A CR at the very end may be the first half of a CRLF still on its way.
      if (this.#buffer[at] === "\r" && at === this.#buffer.length - 1) {
        return out;
      }
      const width = this.#buffer.startsWith("\r\n", at) ? 2 : 1;
      const line = this.#buffer.slice(0, at);
      this.#buffer = this.#buffer.slice(at + width);
      const event = this.#line(line);
      if (event !== undefined) {
        out.push(event);
      }
    }
  }

  #line(line: string): SseEvent | undefined {
    if (line === "") {
      if (this.#data.length === 0) {
        this.#event = "";
        return undefined;
      }
      const event = {
        event: this.#event === "" ? "message" : this.#event,
        data: this.#data.join("\n"),
      };
      this.#data = [];
      this.#dataBytes = 0;
      this.#event = "";
      return event;
    }
    if (line.startsWith(":")) {
      return undefined;
    }
    const colon = line.indexOf(":");
    const field = colon < 0 ? line : line.slice(0, colon);
    let value = colon < 0 ? "" : line.slice(colon + 1);
    if (value.startsWith(" ")) {
      value = value.slice(1);
    }
    if (field === "data") {
      this.#dataBytes += value.length + 1;
      if (this.#dataBytes > MAX_EVENT) {
        throw new TooLargeError();
      }
      this.#data.push(value);
    } else if (field === "event") {
      this.#event = value;
    }
    return undefined;
  }
}

/** The error an envelope stands for, its status from the code (`problem.json`). */
export function envelopeError(envelope: unknown, requestId: string): ApiError {
  const code =
    typeof envelope === "object" &&
    envelope !== null &&
    typeof (envelope as { code?: unknown }).code === "string"
      ? (envelope as { code: string }).code
      : "internal";
  const status = (CODES as Readonly<Record<string, number>>)[code] ?? 500;
  return new ApiError(
    new RawResponse({
      status,
      headers: new Headers(),
      body: new TextEncoder().encode(JSON.stringify(envelope)),
      requestId,
      attempts: 1,
    }),
  );
}

/** What the socket needs from its client. */
export interface SocketConfig {
  /** `wss://host/v1/ws` (or `ws://` on loopback). */
  readonly url: string;
  /** The API's host, for error messages. */
  readonly host: string;
  /** A current access token. */
  token(): Promise<string>;
  /** Forgets the current token, so the next one is fresh. */
  invalidate(): Promise<void>;
  /** Reconnects after a failure, at most this many in a row. */
  readonly maxRetries: number;
  /** Headers each upgrade sends besides the token: the request id and the user agent. */
  headers?(): Record<string, string>;
  /** Asks the retry budget for a reconnect; `false` gives up. */
  reconnect?(): boolean;
}

type WsLike = {
  readyState: number;
  send(data: string): void;
  close(code?: number, reason?: string): void;
  addEventListener(
    type: string,
    listener: (event: { data?: unknown; code?: number }) => void,
  ): void;
};

type WsConstructor = new (url: string, init: { headers: Record<string, string> }) => WsLike;

const DONE = Symbol("done");

/** One stream on the socket: a call, its queue and its reader. */
class SocketCall {
  id = "";
  readonly #queue: unknown[] = [];
  #ended = false;
  #error: unknown;
  #wake: (() => void) | undefined;

  constructor(
    readonly method: string,
    readonly body: unknown,
  ) {}

  get finished(): boolean {
    return this.#ended || this.#error !== undefined;
  }

  data(body: unknown, overflow: () => void): void {
    if (this.finished) {
      return;
    }
    if (this.#queue.length >= SOCKET_QUEUE) {
      overflow();
      return;
    }
    this.#queue.push(body);
    this.#notify();
  }

  end(): void {
    this.#ended = true;
    this.#notify();
  }

  fail(error: unknown): void {
    if (!this.finished) {
      this.#error = error;
      this.#notify();
    }
  }

  #notify(): void {
    const wake = this.#wake;
    this.#wake = undefined;
    wake?.();
  }

  /** The next item, `DONE` after `end`, or the error. */
  async next(signal: AbortSignal | undefined): Promise<unknown> {
    for (;;) {
      signal?.throwIfAborted();
      if (this.#queue.length > 0) {
        return this.#queue.shift();
      }
      if (this.#error !== undefined) {
        throw this.#error;
      }
      if (this.#ended) {
        return DONE;
      }
      await new Promise<void>((resolve, reject) => {
        this.#wake = resolve;
        signal?.addEventListener("abort", () => reject(signal.reason), { once: true });
      });
    }
  }
}

/**
 * The one `/v1/ws` connection a client's streams share. Opened with the first stream,
 * closed after the last; reconnected (with the calls not yet ended issued again) when the
 * server ends it for any reason but a revoked key.
 */
export class SocketHub {
  readonly #config: SocketConfig;
  readonly #calls = new Map<string, SocketCall>();
  #ws: WsLike | undefined;
  #opening: Promise<void> | undefined;
  #next = 0;
  #retries = 0;

  constructor(config: SocketConfig) {
    this.#config = config;
  }

  /** Items of one call: `method` with `body`, until `end`, an error, or the caller stops. */
  async *call(
    method: string,
    body: unknown,
    signal?: AbortSignal,
  ): AsyncGenerator<unknown, void, undefined> {
    const call = new SocketCall(method, body);
    call.id = this.#id();
    this.#calls.set(call.id, call);
    try {
      await this.#ensure();
      this.#issue(call);
      for (;;) {
        const item = await call.next(signal);
        if (item === DONE) {
          return;
        }
        yield item;
      }
    } finally {
      this.#stop(call);
    }
  }

  #id(): string {
    this.#next += 1;
    return String(this.#next);
  }

  #issue(call: SocketCall): void {
    if (this.#ws?.readyState === 1 && !call.finished) {
      this.#ws.send(
        JSON.stringify(
          { type: "call", id: call.id, method: call.method, body: call.body },
          (_, v: unknown) => (typeof v === "bigint" ? v.toString() : v),
        ),
      );
    }
  }

  #stop(call: SocketCall): void {
    if (this.#calls.get(call.id) !== call) {
      return;
    }
    this.#calls.delete(call.id);
    if (!call.finished && this.#ws?.readyState === 1) {
      this.#ws.send(JSON.stringify({ type: "cancel", id: call.id }));
    }
    if (this.#calls.size === 0) {
      const ws = this.#ws;
      this.#ws = undefined;
      ws?.close(1000);
    }
  }

  #ensure(): Promise<void> {
    if (this.#ws?.readyState === 1) {
      return Promise.resolve();
    }
    this.#opening ??= this.#connect().finally(() => {
      this.#opening = undefined;
    });
    return this.#opening;
  }

  async #connect(): Promise<void> {
    let refreshed = false;
    for (let attempt = 0; ; attempt++) {
      const token = await this.#config.token();
      try {
        this.#ws = await this.#open(token);
        return;
      } catch (e) {
        // The WebSocket API hides the refusal's status: one fresh token, as after a 401,
        // then the retry budget with backoff.
        if (!refreshed) {
          refreshed = true;
          await this.#config.invalidate();
          continue;
        }
        if (attempt >= this.#config.maxRetries || this.#config.reconnect?.() === false) {
          throw e;
        }
        await sleep(backoffMs(attempt));
      }
    }
  }

  #open(token: string): Promise<WsLike> {
    const Ws = (globalThis as unknown as { WebSocket?: WsConstructor }).WebSocket;
    if (Ws === undefined) {
      return Promise.reject(
        new ConnectionError(this.#config.host, "this runtime has no WebSocket"),
      );
    }
    return new Promise((resolve, reject) => {
      let ws: WsLike;
      try {
        ws = new Ws(this.#config.url, {
          headers: { ...this.#config.headers?.(), authorization: `Bearer ${token}` },
        });
      } catch (e) {
        reject(new ConnectionError(this.#config.host, String(e), { cause: e }));
        return;
      }
      let open = false;
      ws.addEventListener("open", () => {
        open = true;
        resolve(ws);
      });
      ws.addEventListener("error", () => {
        if (!open) {
          reject(new ConnectionError(this.#config.host, "the socket did not open"));
        }
      });
      ws.addEventListener("message", (event) => this.#message(ws, event.data));
      ws.addEventListener("close", () => {
        if (open) {
          void this.#closed(ws);
        }
      });
    });
  }

  #message(ws: WsLike, data: unknown): void {
    if (ws !== this.#ws || typeof data !== "string") {
      return;
    }
    let frame: { type?: unknown; id?: unknown; body?: unknown; code?: unknown };
    try {
      frame = JSON.parse(data) as typeof frame;
    } catch {
      return;
    }
    this.#retries = 0;
    if (typeof frame.id !== "string") {
      if (frame.type === "error") {
        if (frame.code === "unauthenticated") {
          const error = envelopeError(frame, "");
          for (const call of this.#calls.values()) {
            call.fail(error);
          }
          this.#ws = undefined;
          ws.close(1000);
        } else {
          // The server ends the socket (its longest life): open another.
          this.#ws = undefined;
          ws.close(1000);
          void this.#reconnect();
        }
      }
      return;
    }
    const call = this.#calls.get(frame.id);
    if (call === undefined) {
      return;
    }
    switch (frame.type) {
      case "data":
        call.data(frame.body, () => {
          call.fail(
            new ConnectionError(
              this.#config.host,
              `the stream's reader fell ${SOCKET_QUEUE} items behind; the stream was stopped`,
            ),
          );
          ws.send(JSON.stringify({ type: "cancel", id: call.id }));
        });
        break;
      case "end":
        call.end();
        break;
      case "error":
        call.fail(envelopeError(frame, ""));
        break;
      default:
        break;
    }
  }

  async #closed(ws: WsLike): Promise<void> {
    if (ws !== this.#ws) {
      return;
    }
    this.#ws = undefined;
    await this.#reconnect();
  }

  async #reconnect(): Promise<void> {
    const pending = [...this.#calls.values()].filter((c) => !c.finished);
    if (pending.length === 0) {
      return;
    }
    if (this.#retries > this.#config.maxRetries || this.#config.reconnect?.() === false) {
      const error = new ConnectionError(this.#config.host, "the socket closed and did not reopen");
      for (const call of pending) {
        call.fail(error);
      }
      return;
    }
    this.#retries += 1;
    try {
      await this.#ensure();
    } catch (e) {
      const error =
        e instanceof InOrbitError ? e : new ConnectionError(this.#config.host, String(e));
      for (const call of pending) {
        call.fail(error);
      }
      return;
    }
    // Every call that had not ended goes again, under a new id: streams are reads.
    for (const call of pending) {
      if (this.#calls.get(call.id) === call && !call.finished) {
        this.#calls.delete(call.id);
        call.id = this.#id();
        this.#calls.set(call.id, call);
        this.#issue(call);
      }
    }
  }
}
