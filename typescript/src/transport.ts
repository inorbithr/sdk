/**
 * The Node transport (config.md sections 6.1 to 6.4): a `fetch` over `node:https` for
 * what the global `fetch` cannot be told per client without the `undici` package: a
 * proxy (CONNECT, `no_proxy`, the loopback rule), a CA bundle added to the trust store,
 * a client certificate, key pinning and the connect timeout. Node, Deno and Bun; the
 * built-ins are reached through `process.getBuiltinModule`, never imported.
 *
 * @module
 */

import { ConfigError } from "./errors.js";
import { builtin, fs } from "./platform.js";
import { type NoProxy, type ProxyChoice, parseIp, proxyFor } from "./proxy.js";

/** The settings the transport owns. */
export interface TransportSettings {
  readonly proxy: ProxyChoice | undefined;
  readonly noProxy: NoProxy;
  readonly caBundle: string | undefined;
  readonly systemTrust: boolean;
  readonly clientCert: string | undefined;
  readonly clientKey: string | undefined;
  readonly clientKeyPassword: string | undefined;
  readonly pinnedKeys: readonly string[] | undefined;
  readonly connectTimeout: number;
}

/** The connect timeout the global `fetch` already keeps (undici's default). */
export const DEFAULT_CONNECT_TIMEOUT = 10_000;

/** Whether `settings` need this transport rather than the global `fetch`. */
export function needsNodeTransport(settings: TransportSettings): boolean {
  return (
    (settings.proxy !== undefined && settings.proxy.url !== "off") ||
    settings.caBundle !== undefined ||
    settings.clientCert !== undefined ||
    settings.pinnedKeys !== undefined ||
    !settings.systemTrust ||
    settings.connectTimeout !== DEFAULT_CONNECT_TIMEOUT
  );
}

interface Emitter {
  on(event: string, listener: (...args: never[]) => void): unknown;
  once(event: string, listener: (...args: never[]) => void): unknown;
}

interface Socket extends Emitter {
  write(data: string): boolean;
  destroy(error?: Error): void;
  removeListener(event: string, listener: (...args: never[]) => void): unknown;
  unshift(chunk: Uint8Array): void;
}

interface IncomingMessage extends Emitter {
  readonly statusCode?: number;
  readonly statusMessage?: string;
  readonly rawHeaders: readonly string[];
  pause(): void;
  resume(): void;
  destroy(error?: Error): void;
}

interface ClientRequest extends Emitter {
  end(body?: Uint8Array): void;
  destroy(error?: Error): void;
}

interface HttpModule {
  Agent: new (
    options: Record<string, unknown>,
  ) => {
    createConnection: unknown;
  };
  request(
    url: URL,
    options: Record<string, unknown>,
    callback: (res: IncomingMessage) => void,
  ): ClientRequest;
}

interface PeerCert {
  readonly raw: Uint8Array;
  readonly issuerCertificate?: PeerCert;
}

interface TlsModule {
  connect(options: Record<string, unknown>): Socket;
  readonly rootCertificates: readonly string[];
  getCACertificates?(kind: string): readonly string[];
  checkServerIdentity(host: string, cert: PeerCert): Error | undefined;
}

interface NetModule {
  connect(options: Record<string, unknown>): Socket;
}

interface CryptoModule {
  X509Certificate: new (
    raw: Uint8Array,
  ) => {
    readonly publicKey: { export(o: { type: string; format: string }): Uint8Array };
  };
  createHash(alg: string): {
    update(d: Uint8Array): { digest(enc: string): string };
  };
}

function read(path: string, what: string): Uint8Array {
  const f = fs();
  if (f === undefined) {
    throw new ConfigError(`${what}: this runtime has no file system`);
  }
  try {
    return f.readFileSync(path);
  } catch {
    throw new ConfigError(`${what}: cannot read ${path}`);
  }
}

/** Reads the first answer line and headers of a CONNECT, then hands the socket on. */
function tunnel(socket: Socket, target: string, auth: string | undefined): Promise<void> {
  return new Promise((resolve, reject) => {
    let buffered = new Uint8Array();
    const onData = (chunk: Uint8Array): void => {
      const joined = new Uint8Array(buffered.length + chunk.length);
      joined.set(buffered);
      joined.set(chunk, buffered.length);
      buffered = joined;
      const text = new TextDecoder("latin1").decode(buffered);
      const end = text.indexOf("\r\n\r\n");
      if (end < 0) {
        return;
      }
      socket.removeListener("data", onData as never);
      socket.removeListener("error", onError as never);
      const status = /^HTTP\/1\.[01] (\d{3})/.exec(text)?.[1];
      if (status !== "200") {
        reject(new Error(`the proxy refused the tunnel (HTTP ${status ?? "?"})`));
        return;
      }
      const rest = buffered.subarray(end + 4);
      if (rest.length > 0) {
        socket.unshift(rest);
      }
      resolve();
    };
    const onError = (e: Error): void => reject(e);
    socket.on("data", onData as never);
    socket.once("error", onError as never);
    const authLine = auth === undefined ? "" : `Proxy-Authorization: ${auth}\r\n`;
    socket.write(`CONNECT ${target} HTTP/1.1\r\nHost: ${target}\r\n${authLine}\r\n`);
  });
}

function bracket(host: string): string {
  return host.includes(":") && !host.startsWith("[") ? `[${host}]` : host;
}

/**
 * A `fetch` over `node:http` and `node:https` with `settings`, or a {@link ConfigError}
 * when the runtime has no such modules.
 */
export function nodeFetch(settings: TransportSettings): typeof fetch {
  const http = builtin<HttpModule>("node:http");
  const https = builtin<HttpModule>("node:https");
  const tls = builtin<TlsModule>("node:tls");
  const net = builtin<NetModule>("node:net");
  const crypto = builtin<CryptoModule>("node:crypto");
  if (
    http === undefined ||
    https === undefined ||
    tls === undefined ||
    net === undefined ||
    crypto === undefined
  ) {
    throw new ConfigError(
      "proxy, ca_bundle, client certificates, pinning and connect_timeout need Node's http and tls modules, which this runtime does not have; pass your own fetch instead",
    );
  }
  const trust: Record<string, unknown> = {};
  if (settings.caBundle !== undefined) {
    const bundle = new TextDecoder().decode(read(settings.caBundle, "ca_bundle"));
    const system = settings.systemTrust
      ? (tls.getCACertificates?.("default") ?? tls.rootCertificates)
      : [];
    trust.ca = [...system, bundle];
  }
  if (settings.clientCert !== undefined && settings.clientKey !== undefined) {
    trust.cert = read(settings.clientCert, "client_cert");
    trust.key = read(settings.clientKey, "client_key");
    if (settings.clientKeyPassword !== undefined) {
      trust.passphrase = settings.clientKeyPassword;
    }
  }
  const pins = settings.pinnedKeys;
  if (pins !== undefined) {
    trust.checkServerIdentity = (host: string, cert: PeerCert): Error | undefined => {
      const base = tls.checkServerIdentity(host, cert);
      if (base !== undefined) {
        return base;
      }
      for (let c: PeerCert | undefined = cert; c !== undefined; c = c.issuerCertificate) {
        const spki = new crypto.X509Certificate(c.raw).publicKey.export({
          type: "spki",
          format: "der",
        });
        if (pins.includes(crypto.createHash("sha256").update(spki).digest("base64"))) {
          return undefined;
        }
        if (c.issuerCertificate === c) {
          break;
        }
      }
      return new Error(`no certificate ${host} presented matches a pinned key`);
    };
  }

  const connect = (
    secure: boolean,
    options: { host?: string; hostname?: string; port?: number | string; servername?: string },
    done: (err: Error | null, socket?: Socket) => void,
  ): Socket | undefined => {
    const host = String(options.hostname ?? options.host ?? "localhost").replace(/^\[|\]$/g, "");
    const port = Number(options.port ?? (secure ? 443 : 80));
    const servername = parseIp(host) === undefined ? host : undefined;
    const target = new URL(`${secure ? "https" : "http"}://${bracket(host)}:${port}/`);
    const proxy = proxyFor(target, settings.proxy, settings.noProxy);
    let finished = false;
    let current: Socket | undefined;
    const timer = setTimeout(() => {
      current?.destroy(
        new Error(`connecting took longer than ${Math.round(settings.connectTimeout / 1000)} s`),
      );
    }, settings.connectTimeout);
    const ready = (): void => clearTimeout(timer);
    const fail = (e: Error): void => {
      clearTimeout(timer);
      if (!finished) {
        finished = true;
        done(e);
      }
    };
    const secureOver = (socket?: Socket): Socket => {
      const s = tls.connect({
        ...trust,
        ...(socket === undefined ? { host, port } : { socket }),
        ...(servername === undefined ? {} : { servername }),
        ALPNProtocols: ["http/1.1"],
      });
      s.once("secureConnect", ready);
      return s;
    };
    if (proxy === undefined) {
      if (secure) {
        current = secureOver();
      } else {
        current = net.connect({ host, port });
        current.once("connect", ready);
      }
      current.once("error", () => clearTimeout(timer));
      finished = true;
      return current;
    }
    const p = new URL(proxy);
    const auth =
      p.username === "" && p.password === ""
        ? undefined
        : `Basic ${btoa(`${decodeURIComponent(p.username)}:${decodeURIComponent(p.password)}`)}`;
    const proxyHost = p.hostname.replace(/^\[|\]$/g, "");
    const proxyPort = Number(p.port || (p.protocol === "https:" ? 443 : 80));
    const raw =
      p.protocol === "https:"
        ? tls.connect({
            ...(trust.ca === undefined ? {} : { ca: trust.ca }),
            host: proxyHost,
            port: proxyPort,
            ...(parseIp(proxyHost) === undefined ? { servername: proxyHost } : {}),
          })
        : net.connect({ host: proxyHost, port: proxyPort });
    current = raw;
    raw.once("error", fail as never);
    raw.once(p.protocol === "https:" ? "secureConnect" : "connect", () => {
      tunnel(raw, `${bracket(host)}:${port}`, auth).then(
        () => {
          if (!secure) {
            ready();
            finished = true;
            done(null, raw);
            return;
          }
          const s = secureOver(raw);
          current = s;
          s.once("secureConnect", () => {
            if (!finished) {
              finished = true;
              done(null, s);
            }
          });
          s.once("error", fail as never);
        },
        (e: Error) => {
          raw.destroy();
          fail(e);
        },
      );
    });
    return undefined;
  };

  const agent = (secure: boolean): unknown => {
    const a = new (secure ? https : http).Agent({ keepAlive: true });
    a.createConnection = (
      options: Record<string, unknown>,
      done: (err: Error | null, socket?: Socket) => void,
    ): Socket | undefined => connect(secure, options, done);
    return a;
  };
  const agents = { http: agent(false), https: agent(true) };

  return (input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    const url = new URL(input instanceof Request ? input.url : String(input));
    const secure = url.protocol === "https:";
    const headers = new Headers(init?.headers);
    let body: Uint8Array | undefined;
    const b = init?.body;
    if (typeof b === "string") {
      body = new TextEncoder().encode(b);
    } else if (b instanceof URLSearchParams) {
      body = new TextEncoder().encode(b.toString());
      if (!headers.has("content-type")) {
        headers.set("content-type", "application/x-www-form-urlencoded;charset=UTF-8");
      }
    } else if (b instanceof Uint8Array) {
      body = b;
    } else if (b instanceof ArrayBuffer) {
      body = new Uint8Array(b);
    } else if (b !== undefined && b !== null) {
      return Promise.reject(new TypeError("this transport sends strings, forms and bytes only"));
    }
    if (body !== undefined) {
      headers.set("content-length", String(body.length));
    }
    const signal = init?.signal ?? undefined;
    const method = init?.method ?? "GET";
    return new Promise<Response>((resolve, reject) => {
      if (signal?.aborted) {
        reject(signal.reason);
        return;
      }
      let res: IncomingMessage | undefined;
      let stream: ReadableStreamDefaultController<Uint8Array> | undefined;
      const req = (secure ? https : http).request(
        url,
        {
          method,
          headers: Object.fromEntries(headers),
          agent: secure ? agents.https : agents.http,
        },
        (r) => {
          res = r;
          const out = new Headers();
          for (let i = 0; i + 1 < r.rawHeaders.length; i += 2) {
            out.append(r.rawHeaders[i] ?? "", r.rawHeaders[i + 1] ?? "");
          }
          const status = r.statusCode ?? 0;
          const empty = method === "HEAD" || status === 204 || status === 205 || status === 304;
          const readable = empty
            ? null
            : new ReadableStream<Uint8Array>({
                start(c) {
                  stream = c;
                  r.on("data", ((chunk: Uint8Array) => {
                    c.enqueue(new Uint8Array(chunk));
                    if ((c.desiredSize ?? 1) <= 0) {
                      r.pause();
                    }
                  }) as never);
                  r.on("end", () => {
                    try {
                      c.close();
                    } catch {
                      // already errored
                    }
                  });
                  r.on("error", ((e: Error) => {
                    try {
                      c.error(signal?.aborted ? signal.reason : e);
                    } catch {
                      // already closed
                    }
                  }) as never);
                },
                pull() {
                  r.resume();
                },
                cancel() {
                  r.destroy();
                },
              });
          if (empty) {
            r.resume();
          }
          resolve(
            new Response(readable, {
              status,
              statusText: r.statusMessage ?? "",
              headers: out,
            }),
          );
        },
      );
      req.on("error", ((e: Error) => {
        reject(signal?.aborted ? signal.reason : new TypeError("fetch failed", { cause: e }));
      }) as never);
      signal?.addEventListener(
        "abort",
        () => {
          req.destroy();
          res?.destroy();
          try {
            stream?.error(signal.reason);
          } catch {
            // already closed
          }
          reject(signal.reason);
        },
        { once: true },
      );
      req.end(body);
    });
  };
}
