// @@NAME@@: a command line with text and JSON output, exit codes that mean something, and a
// version that says which commit it was built from. Node's standard library only.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

export const VERSION = "0.1.0";

/** Exit codes: the contract scripts rely on. */
export const EXIT_OK = 0;
export const EXIT_FAILURE = 1;
export const EXIT_USAGE = 2;

export const USAGE = `Usage: @@NAME@@ [--json] <command>

Commands:
  greet [NAME]   Say hello to NAME (default: world)
  version        Print the version and the commit it was built from

Options:
      --json     Machine-readable output
  -h, --help     Show this help

Exit codes: 0 success, 1 failure, 2 usage error.
`;

/**
 * The commit this runs from: a `commit.txt` stamped next to the source by a release build,
 * else the checkout's git head, else "unknown".
 */
export function buildCommit(): string {
  const valid = (s: string): string | undefined => (/^[0-9a-f]{7,40}$/.test(s.trim()) ? s.trim().slice(0, 12) : undefined);
  try {
    const stamped = valid(readFileSync(new URL("./commit.txt", import.meta.url), "utf8"));
    if (stamped) return stamped;
  } catch {
    // no stamp: fall through
  }
  try {
    const head = valid(
      execFileSync("git", ["rev-parse", "--short=12", "HEAD"], {
        cwd: new URL(".", import.meta.url).pathname,
        stdio: ["ignore", "pipe", "ignore"],
        timeout: 2000,
      }).toString(),
    );
    if (head) return head;
  } catch {
    // no git, or not a checkout
  }
  return "unknown";
}

export interface Out {
  out: (s: string) => void;
  err: (s: string) => void;
}

/** Keeps a hostile argument from shaping the error line: it is cut and escaped. */
export function quote(s: string): string {
  const chars = [...s];
  const shown = chars.length > 40 ? chars.slice(0, 40).join("") + "..." : s;
  let b = '"';
  for (const c of shown) {
    const n = c.codePointAt(0) ?? 0;
    if (c === '"' || c === "\\") b += "\\" + c;
    else if (n < 0x20 || n === 0x7f) b += "\\x" + n.toString(16).padStart(2, "0");
    else b += c;
  }
  return b + '"';
}

function emit(io: Out, v: Record<string, string>): void {
  // keys sorted: stable output for scripts
  const sorted = Object.fromEntries(Object.entries(v).sort(([a], [b]) => (a < b ? -1 : 1)));
  io.out(JSON.stringify(sorted) + "\n");
}

function usageError(io: Out, msg: string): number {
  io.err(`@@NAME@@: ${msg}\nrun \`@@NAME@@ --help\` for the commands\n`);
  return EXIT_USAGE;
}

/** The whole program, with its output as arguments so a test can read it. */
export function run(args: string[], io: Out, commit: string = buildCommit()): number {
  let asJson = false;
  const rest: string[] = [];
  for (const a of args) {
    if (a === "--json") asJson = true;
    else if (a === "-h" || a === "--help") {
      io.out(USAGE);
      return EXIT_OK;
    } else rest.push(a);
  }
  if (rest.length === 0) {
    io.err(USAGE);
    return EXIT_USAGE;
  }
  switch (rest[0]) {
    case "greet": {
      if (rest.length > 2) return usageError(io, "greet takes at most one name");
      const name = rest.length === 2 ? rest[1]!.trim() : "world";
      if (name === "" || [...name].length > 64) {
        io.err("@@NAME@@: the name is 1 to 64 characters of text\n");
        return EXIT_FAILURE;
      }
      const message = "Hello, " + name;
      if (asJson) emit(io, { message });
      else io.out(message + "\n");
      return EXIT_OK;
    }
    case "version":
    case "--version":
      if (rest.length > 1) return usageError(io, "version takes no arguments");
      if (asJson) emit(io, { name: "@@NAME@@", version: VERSION, commit });
      else io.out(`@@NAME@@ ${VERSION} (commit ${commit})\n`);
      return EXIT_OK;
    default:
      return usageError(io, "unknown command " + quote(rest[0]!));
  }
}
