/**
 * Which proxy a request goes through (config.md section 6.2): the one `no_proxy` grammar
 * every runtime implements itself, and the loopback rule.
 *
 * @module
 */

interface Ip {
  readonly v6: boolean;
  readonly value: bigint;
}

function parseV4(s: string): bigint | undefined {
  const parts = s.split(".");
  if (parts.length !== 4) {
    return undefined;
  }
  let n = 0n;
  for (const p of parts) {
    if (!/^\d{1,3}$/.test(p) || (p.length > 1 && p.startsWith("0")) || Number(p) > 255) {
      return undefined;
    }
    n = (n << 8n) | BigInt(Number(p));
  }
  return n;
}

function parseV6(s: string): bigint | undefined {
  if (s.includes("%")) {
    return undefined;
  }
  let tail: bigint[] = [];
  let text = s;
  const lastColon = text.lastIndexOf(":");
  if (text.includes(".") && lastColon >= 0) {
    const v4 = parseV4(text.slice(lastColon + 1));
    if (v4 === undefined) {
      return undefined;
    }
    tail = [v4 >> 16n, v4 & 0xffffn];
    text = `${text.slice(0, lastColon + 1)}x`;
  }
  const halves = text.split("::");
  if (halves.length > 2) {
    return undefined;
  }
  const group = (h: string): bigint[] | undefined => {
    if (h === "") {
      return [];
    }
    const out: bigint[] = [];
    for (const g of h.split(":")) {
      if (g === "x") {
        out.push(...tail);
      } else if (/^[0-9a-f]{1,4}$/i.test(g)) {
        out.push(BigInt(Number.parseInt(g, 16)));
      } else {
        return undefined;
      }
    }
    return out;
  };
  const head = group(halves[0] ?? "");
  const rest = halves.length === 2 ? group(halves[1] ?? "") : [];
  if (head === undefined || rest === undefined) {
    return undefined;
  }
  let groups: bigint[];
  if (halves.length === 2) {
    const fill = 8 - head.length - rest.length;
    if (fill < 1) {
      return undefined;
    }
    groups = [...head, ...Array.from({ length: fill }, () => 0n), ...rest];
  } else {
    groups = head;
  }
  if (groups.length !== 8) {
    return undefined;
  }
  return groups.reduce((acc, g) => (acc << 16n) | g, 0n);
}

/** An IP literal, with or without brackets around an IPv6 one. */
export function parseIp(raw: string): Ip | undefined {
  const s = raw.startsWith("[") && raw.endsWith("]") ? raw.slice(1, -1) : raw;
  const v4 = parseV4(s);
  if (v4 !== undefined) {
    return { v6: false, value: v4 };
  }
  if (s.includes(":")) {
    const v6 = parseV6(s);
    return v6 === undefined ? undefined : { v6: true, value: v6 };
  }
  return undefined;
}

/** Whether `host` (a URL's hostname) is this machine. */
export function isLoopback(host: string): boolean {
  const h = host.toLowerCase();
  if (h === "localhost") {
    return true;
  }
  const ip = parseIp(h);
  if (ip === undefined) {
    return false;
  }
  return ip.v6 ? ip.value === 1n : ip.value >> 24n === 127n;
}

type Entry =
  | { readonly kind: "all" }
  | { readonly kind: "cidr"; readonly v6: boolean; readonly net: bigint; readonly bits: number }
  | { readonly kind: "ip"; readonly ip: Ip; readonly port: number | undefined }
  | { readonly kind: "name"; readonly name: string; readonly port: number | undefined };

/** One `no_proxy` entry, or `undefined` when it fits none of the forms. */
export function parseNoProxyEntry(raw: string): Entry | undefined {
  const e = raw.trim().toLowerCase();
  if (e === "*") {
    return { kind: "all" };
  }
  const slash = e.indexOf("/");
  if (slash >= 0) {
    const bitsText = e.slice(slash + 1);
    const ip = parseIp(e.slice(0, slash));
    if (ip === undefined || !/^\d{1,3}$/.test(bitsText)) {
      return undefined;
    }
    const bits = Number(bitsText);
    const width = ip.v6 ? 128 : 32;
    if (bits > width) {
      return undefined;
    }
    const shift = BigInt(width - bits);
    return { kind: "cidr", v6: ip.v6, net: ip.value >> shift, bits };
  }
  const bare = parseIp(e);
  if (bare !== undefined) {
    return { kind: "ip", ip: bare, port: undefined };
  }
  let host = e;
  let port: number | undefined;
  const colon = e.lastIndexOf(":");
  if (colon >= 0) {
    const h = e.slice(0, colon);
    if (!h.includes(":") || h.endsWith("]")) {
      const p = e.slice(colon + 1);
      if (!/^\d{1,5}$/.test(p) || Number(p) > 65535) {
        return undefined;
      }
      host = h;
      port = Number(p);
    }
  }
  const ip = parseIp(host);
  if (ip !== undefined) {
    return { kind: "ip", ip, port };
  }
  const name = host.replace(/^\.+/, "");
  if (name === "" || !name.split(".").every((l) => /^[a-z0-9_-]+$/.test(l))) {
    return undefined;
  }
  return { kind: "name", name, port };
}

/** A parsed `no_proxy` list; `undefined` entries were refused at load. */
export type NoProxy = readonly Entry[];

/** Parses a list; throws the first entry that fits no form. */
export function parseNoProxy(entries: readonly string[]): NoProxy {
  const out: Entry[] = [];
  for (const raw of entries) {
    if (raw.trim() === "") {
      continue;
    }
    const e = parseNoProxyEntry(raw);
    if (e === undefined) {
      throw new Error(raw);
    }
    out.push(e);
  }
  return out;
}

function defaultPort(url: URL): number {
  if (url.port !== "") {
    return Number(url.port);
  }
  return url.protocol === "http:" || url.protocol === "ws:" ? 80 : 443;
}

/** Whether `url` is left out of the proxy by `noProxy`. */
export function bypasses(url: URL, noProxy: NoProxy): boolean {
  const host = url.hostname.toLowerCase();
  const port = defaultPort(url);
  const ip = parseIp(host);
  return noProxy.some((e) => {
    switch (e.kind) {
      case "all":
        return true;
      case "cidr":
        return (
          ip !== undefined &&
          ip.v6 === e.v6 &&
          ip.value >> BigInt((e.v6 ? 128 : 32) - e.bits) === e.net
        );
      case "ip":
        return (
          ip !== undefined &&
          ip.v6 === e.ip.v6 &&
          ip.value === e.ip.value &&
          (e.port === undefined || e.port === port)
        );
      case "name":
        return (
          (host === e.name || host.endsWith(`.${e.name}`)) &&
          (e.port === undefined || e.port === port)
        );
      default:
        return false;
    }
  });
}

/** The proxy in effect: its URL, and whether it was set on purpose (code, INORBIT_PROXY, the file). */
export interface ProxyChoice {
  readonly url: string;
  readonly explicit: boolean;
}

/** The proxy `url` goes through, or `undefined` for a direct connection. */
export function proxyFor(
  url: URL,
  proxy: ProxyChoice | undefined,
  noProxy: NoProxy,
): string | undefined {
  if (proxy === undefined || proxy.url === "off") {
    return undefined;
  }
  if (!proxy.explicit && isLoopback(url.hostname)) {
    return undefined;
  }
  return bypasses(url, noProxy) ? undefined : proxy.url;
}
