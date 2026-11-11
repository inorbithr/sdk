// A small, safe Markdown reader for Decisions documents (RFC 0217): it makes a tree of blocks and inline runs and the
// page draws that tree as React elements, so no HTML string is ever set and a link is only drawn when it is http(s),
// mailto or an in-page anchor. Front matter (`---` ... `---`) is split off first. No React, no DOM, so node tests it.

export type Inline =
  | { t: "text"; v: string }
  | { t: "code"; v: string }
  | { t: "strong"; v: Inline[] }
  | { t: "em"; v: Inline[] }
  | { t: "link"; href: string; v: Inline[] };

export type Block =
  | { t: "heading"; level: number; id: string; v: Inline[] }
  | { t: "para"; v: Inline[] }
  | { t: "code"; lang: string; v: string }
  | { t: "quote"; v: Block[] }
  | { t: "list"; ordered: boolean; items: Inline[][] }
  | { t: "table"; head: Inline[][]; rows: Inline[][][] }
  | { t: "rule" };

export type Parsed = { front: [string, string][]; blocks: Block[]; toc: { level: number; id: string; text: string }[] };

/** Front matter: the `key: value` lines between two `---` fences at the very top. */
export function splitFront(src: string): { front: [string, string][]; body: string } {
  const m = /^---\r?\n([\s\S]*?)\r?\n---[ \t]*(?:\r?\n|$)/.exec(src);
  if (!m) return { front: [], body: src };
  const front: [string, string][] = [];
  for (const line of (m[1] ?? "").split(/\r?\n/)) {
    const k = /^([A-Za-z0-9_-]+):\s*(.*)$/.exec(line);
    if (k) front.push([k[1] ?? "", (k[2] ?? "").replace(/^["']|["']$/g, "")]);
  }
  return { front, body: src.slice(m[0].length) };
}

export const safeHref = (h: string): string | null => (/^(https?:\/\/|mailto:|#)/i.test(h.trim()) ? h.trim() : null);

export function inline(s: string): Inline[] {
  const out: Inline[] = [];
  const re = /`([^`]+)`|\*\*([^*]+)\*\*|(?<![\w*])\*([^*\s][^*]*)\*(?![\w*])|\[([^\]]+)\]\(([^)\s]+)\)/g;
  let at = 0;
  for (let m = re.exec(s); m !== null; m = re.exec(s)) {
    if (m.index > at) out.push({ t: "text", v: s.slice(at, m.index) });
    if (m[1] !== undefined) out.push({ t: "code", v: m[1] });
    else if (m[2] !== undefined) out.push({ t: "strong", v: inline(m[2]) });
    else if (m[3] !== undefined) out.push({ t: "em", v: inline(m[3]) });
    else {
      const href = safeHref(m[5] ?? "");
      if (href === null) out.push({ t: "text", v: m[4] ?? "" });
      else out.push({ t: "link", href, v: inline(m[4] ?? "") });
    }
    at = m.index + m[0].length;
  }
  if (at < s.length) out.push({ t: "text", v: s.slice(at) });
  return out;
}

export const slug = (s: string): string =>
  s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");

const cells = (line: string): string[] =>
  line
    .trim()
    .replace(/^\||\|$/g, "")
    .split("|")
    .map((c) => c.trim());

export function parseMarkdown(src: string): Parsed {
  const { front, body } = splitFront(src);
  const lines = body.split(/\r?\n/);
  const blocks: Block[] = [];
  const toc: Parsed["toc"] = [];
  const used = new Map<string, number>();
  let i = 0;
  while (i < lines.length) {
    const line = lines[i] ?? "";
    if (line.trim() === "") {
      i++;
      continue;
    }
    const fence = /^```\s*([\w-]*)/.exec(line);
    if (fence) {
      const buf: string[] = [];
      i++;
      while (i < lines.length && !/^```/.test(lines[i] ?? "")) buf.push(lines[i++] ?? "");
      i++;
      blocks.push({ t: "code", lang: fence[1] ?? "", v: buf.join("\n") });
      continue;
    }
    const h = /^(#{1,6})\s+(.*?)\s*#*$/.exec(line);
    if (h) {
      const text = h[2] ?? "";
      const base = slug(text) || "section";
      const n = used.get(base) ?? 0;
      used.set(base, n + 1);
      const id = n === 0 ? base : `${base}-${n}`;
      blocks.push({ t: "heading", level: (h[1] ?? "#").length, id, v: inline(text) });
      toc.push({ level: (h[1] ?? "#").length, id, text });
      i++;
      continue;
    }
    if (/^(-{3,}|\*{3,})\s*$/.test(line)) {
      blocks.push({ t: "rule" });
      i++;
      continue;
    }
    if (/^>/.test(line)) {
      const buf: string[] = [];
      while (i < lines.length && /^>/.test(lines[i] ?? "")) buf.push((lines[i++] ?? "").replace(/^>\s?/, ""));
      blocks.push({ t: "quote", v: parseMarkdown(buf.join("\n")).blocks });
      continue;
    }
    if (/^\|.*\|\s*$/.test(line) && /^\|?\s*:?-{2,}/.test(lines[i + 1] ?? "")) {
      const head = cells(line).map(inline);
      i += 2;
      const rows: Inline[][][] = [];
      while (i < lines.length && /^\|.*\|\s*$/.test(lines[i] ?? "")) rows.push(cells(lines[i++] ?? "").map(inline));
      blocks.push({ t: "table", head, rows });
      continue;
    }
    const li = /^\s*([-*+]|\d+[.)])\s+/.exec(line);
    if (li) {
      const ordered = /\d/.test(li[1] ?? "");
      const items: Inline[][] = [];
      while (i < lines.length && /^\s*([-*+]|\d+[.)])\s+/.test(lines[i] ?? "")) {
        let item = (lines[i++] ?? "").replace(/^\s*([-*+]|\d+[.)])\s+/, "");
        while (i < lines.length && /^\s{2,}\S/.test(lines[i] ?? "") && !/^\s*([-*+]|\d+[.)])\s+/.test(lines[i] ?? ""))
          item += ` ${(lines[i++] ?? "").trim()}`;
        items.push(inline(item));
      }
      blocks.push({ t: "list", ordered, items });
      continue;
    }
    const buf: string[] = [];
    while (
      i < lines.length &&
      (lines[i] ?? "").trim() !== "" &&
      !/^(#{1,6}\s|```|>|\s*([-*+]|\d+[.)])\s)/.test(lines[i] ?? "")
    )
      buf.push((lines[i++] ?? "").trim());
    blocks.push({ t: "para", v: inline(buf.join(" ")) });
  }
  return { front, blocks, toc };
}
