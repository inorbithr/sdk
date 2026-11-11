import assert from "node:assert/strict";
import { test } from "node:test";
import { parseMarkdown, safeHref, splitFront } from "../src/lib/markdown-core.ts";

test("front matter is split off", () => {
  const r = splitFront('---\ntitle: "A"\nstatus: draft\n---\n# Hi\n');
  assert.deepEqual(r.front, [
    ["title", "A"],
    ["status", "draft"],
  ]);
  assert.equal(r.body, "# Hi\n");
  assert.deepEqual(splitFront("# no front").front, []);
});

test("blocks: headings with unique ids, lists, code, table, quote", () => {
  const p = parseMarkdown(
    "# A\n\ntext **b** `c`\n\n## A\n\n- one\n- two\n\n```rust\nlet x;\n```\n\n| h | i |\n|---|---|\n| 1 | 2 |\n\n> q\n",
  );
  assert.deepEqual(
    p.toc.map((t) => t.id),
    ["a", "a-1"],
  );
  const kinds = p.blocks.map((b) => b.t);
  assert.deepEqual(kinds, ["heading", "para", "heading", "list", "code", "table", "quote"]);
});

test("links: only http, mailto and anchors are drawn", () => {
  assert.equal(safeHref("javascript:alert(1)"), null);
  assert.equal(safeHref("https://x.y"), "https://x.y");
  const p = parseMarkdown("[bad](javascript:x) [ok](#a)");
  const para = p.blocks[0];
  assert.ok(
    para?.t === "para" &&
      para.v.some((x) => x.t === "link") &&
      !para.v.some((x) => x.t === "link" && x.href.startsWith("java")),
  );
});
