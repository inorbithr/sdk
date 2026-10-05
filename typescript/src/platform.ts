/**
 * What the runtime offers beyond the web platform: the environment, files, the home
 * directory and processes. Node, Deno and Bun hand out their built-ins through
 * `process.getBuiltinModule`, so nothing here imports `node:` modules and the same build
 * runs in browsers and Workers, where every answer is "not here".
 *
 * @module
 */

/** An operating system whose conventions the configuration follows (config.md section 4.1). */
export type Os = "linux" | "macos" | "windows";

/** What `stat` answers, as far as this package reads it. */
export interface StatLike {
  readonly mtimeMs: number;
  readonly size: number;
  isFile(): boolean;
}

/** The part of `node:fs` this package uses. */
export interface FsLike {
  readFileSync(path: string): Uint8Array;
  statSync(path: string, options: { throwIfNoEntry: false }): StatLike | undefined;
  readonly promises: {
    readFile(path: string): Promise<Uint8Array>;
    stat(path: string): Promise<StatLike>;
  };
}

interface ProcessLike {
  readonly env?: Record<string, string | undefined>;
  readonly platform?: string;
  readonly arch?: string;
  readonly version?: string;
  readonly versions?: Record<string, string | undefined>;
  cwd?(): string;
  getBuiltinModule?(name: string): unknown;
}

/** The global `process`, when the runtime has one. */
export function proc(): ProcessLike | undefined {
  return (globalThis as { process?: ProcessLike }).process;
}

/** A Node built-in (`node:fs`), or `undefined` where the runtime has none. */
export function builtin<T>(name: string): T | undefined {
  try {
    return proc()?.getBuiltinModule?.(name) as T | undefined;
  } catch {
    return undefined;
  }
}

/** The file system, where there is one. */
export function fs(): FsLike | undefined {
  return builtin<FsLike>("node:fs");
}

/** The process environment, or an empty one. */
export function processEnv(): Readonly<Record<string, string | undefined>> {
  return proc()?.env ?? {};
}

/** The OS this runs on, by the configuration's conventions. */
export function currentOs(): Os {
  const p = proc()?.platform;
  return p === "darwin" ? "macos" : p === "win32" ? "windows" : "linux";
}

/** The home directory, or `null` when there is none. */
export function homeDir(): string | null {
  try {
    const h = builtin<{ homedir(): string }>("node:os")?.homedir();
    return h === undefined || h === "" ? null : h;
  } catch {
    return null;
  }
}

/** The working directory, or `/` where there is none. */
export function workingDir(): string {
  try {
    return proc()?.cwd?.() ?? "/";
  } catch {
    return "/";
  }
}

/** Which JavaScript runtime this is, for the user agent (config.md section 7.6). */
export function runtimeName(): { readonly name: string; readonly version: string } {
  const g = globalThis as { Deno?: { version?: { deno?: string } }; Bun?: { version?: string } };
  if (g.Deno !== undefined) {
    return { name: "deno", version: g.Deno.version?.deno ?? "unknown" };
  }
  if (g.Bun !== undefined) {
    return { name: "bun", version: g.Bun.version ?? "unknown" };
  }
  const v = proc()?.versions?.node;
  if (v !== undefined) {
    return { name: "node", version: v };
  }
  return { name: "browser", version: "unknown" };
}

/** `<os>/<arch>` in the user agent's vocabulary. */
export function osArch(): string {
  const p = proc();
  const os: Record<string, string> = {
    linux: "linux",
    darwin: "macos",
    win32: "windows",
    freebsd: "freebsd",
    android: "android",
  };
  const arch: Record<string, string> = {
    x64: "x86_64",
    arm64: "aarch64",
    ia32: "x86",
    arm: "arm",
    riscv64: "riscv64",
  };
  return `${os[p?.platform ?? ""] ?? "other"}/${arch[p?.arch ?? ""] ?? "other"}`;
}
