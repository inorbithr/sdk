// A unified diff, split by file. Its code is coloured by the shared highlighter (code-highlight.ts).

export type DiffLine = { t: "add" | "del" | "ctx" | "hunk"; text: string };
export type DiffFile = { path: string; additions: number; deletions: number; lines: DiffLine[]; binary: boolean };

/** A path as git prints it: plain, or in double quotes with C escapes (tabs, quotes, backslashes, octal bytes). */
function unquote(s: string): string {
  if (!(s.length >= 2 && s.startsWith('"') && s.endsWith('"'))) return s;
  const bytes: number[] = [];
  const esc: Record<string, number> = { n: 10, t: 9, r: 13, a: 7, b: 8, f: 12, v: 11, '"': 34, "\\": 92 };
  const body = s.slice(1, -1);
  for (let i = 0; i < body.length; i++) {
    const c = body[i]!;
    if (c === "\\" && i + 1 < body.length) {
      const n = body[i + 1]!;
      const oct = /^[0-3][0-7]{2}/.exec(body.slice(i + 1, i + 4));
      if (oct) {
        bytes.push(parseInt(oct[0], 8));
        i += 3;
        continue;
      }
      if (n in esc) {
        bytes.push(esc[n]!);
        i++;
        continue;
      }
    }
    const ch = String.fromCodePoint(body.codePointAt(i)!);
    if (ch.length > 1) i++;
    bytes.push(...new TextEncoder().encode(ch));
  }
  return new TextDecoder().decode(new Uint8Array(bytes));
}

/** The new path of a `diff --git` header (what follows "diff --git "), quoted or not; null when it isn't one. */
function headerPath(rest: string): string | null {
  const q = '"(?:[^"\\\\]|\\\\.)*"';
  const quoted = new RegExp(`^(?:${q}|a/\\S+) (${q})$`).exec(rest);
  if (quoted) return unquote(quoted[1]!).replace(/^b\//, "");
  const plain = /^(?:"(?:[^"\\]|\\.)*"|a\/.+?) b\/(.+)$/.exec(rest);
  return plain ? plain[1]! : null;
}

export function parseDiff(diff: string): DiffFile[] {
  const files: DiffFile[] = [];
  let cur: DiffFile | null = null;
  // Inside a hunk, the lines its header still has to come (old side, new side). Past them, what
  // follows (a format-patch signature, the next file's header) is not part of the change.
  let inHunk = false;
  let left = { old: Infinity, new: Infinity };
  const startHunk = (line: string) => {
    const m = /^@@ -\d+(?:,(\d+))? \+\d+(?:,(\d+))? @@/.exec(line);
    left = m ? { old: m[1] === undefined ? 1 : Number(m[1]), new: m[2] === undefined ? 1 : Number(m[2]) } : { old: Infinity, new: Infinity };
    inHunk = left.old > 0 || left.new > 0;
    cur!.lines.push({ t: "hunk", text: line });
  };
  const all = diff.replace(/\r\n?/g, "\n").split("\n");
  for (let k = 0; k < all.length; k++) {
    const line = all[k]!;
    if (inHunk && cur) {
      if (line.startsWith("\\")) continue;
      if (line.startsWith("@@")) {
        startHunk(line);
        continue;
      }
      if (line.startsWith("+")) {
        cur.additions++;
        cur.lines.push({ t: "add", text: line.slice(1) });
        left.new--;
      } else if (line.startsWith("-")) {
        cur.deletions++;
        cur.lines.push({ t: "del", text: line.slice(1) });
        left.old--;
      } else if (line.startsWith("diff --git ")) {
        inHunk = false;
      } else {
        cur.lines.push({ t: "ctx", text: line.startsWith(" ") ? line.slice(1) : line });
        left.old--;
        left.new--;
      }
      if (inHunk) {
        if (left.old <= 0 && left.new <= 0) inHunk = false;
        continue;
      }
    }
    const head = line.startsWith("diff --git ") ? headerPath(line.slice(11)) : null;
    if (head !== null) {
      cur = { path: head, additions: 0, deletions: 0, lines: [], binary: false };
      files.push(cur);
      continue;
    }
    // A plain unified diff (no "diff --git" line): a file starts at "--- old", "+++ new", "@@".
    if (line.startsWith("--- ") && all[k + 1]?.startsWith("+++ ") && all[k + 2]?.startsWith("@@")) {
      const name = (l: string) => unquote(l.slice(4).split("\t")[0]!.trim());
      const to = name(all[k + 1]!);
      const path = (to === "/dev/null" ? name(line) : to).replace(/^[ab]\//, "");
      if (!cur || cur.lines.length) {
        cur = { path, additions: 0, deletions: 0, lines: [], binary: false };
        files.push(cur);
      }
      k++;
      continue;
    }
    if (!cur) continue;
    if (line.startsWith("@@")) startHunk(line);
    else if (line.startsWith("Binary files") || line.startsWith("GIT binary patch")) cur.binary = true;
    else if (line.startsWith("rename to ")) cur.path = unquote(line.slice(10));
  }
  // The last file's trailing blank line is the split's, not a context line.
  const last = files[files.length - 1]?.lines;
  if (last && last.length && last[last.length - 1]!.t === "ctx" && last[last.length - 1]!.text === "") last.pop();
  return files;
}
