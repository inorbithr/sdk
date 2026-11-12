// Comment anchoring on the canvas (src/canvas/anchors.ts). Run with `pnpm test`.
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  anchorAt,
  countByAnchor,
  headingId,
  locateQuote,
  prsIn,
  quoteFor,
  quoteInSection,
  replaceSection,
  sections,
  splitFrontMatter,
  targetKey,
} from "../src/canvas/anchors.ts";

const DOC = `---
title: A test
status: draft
---

Intro line.

## What the owner gets

- Coordinate from the phone.
- A coordinator that is **always** online.

### The job state machine

\`\`\`
## not a heading
\`\`\`
Intake to Done.

## Problem

The coordinator is one chat on one Mac. See https://github.com/inorbithr/core/pull/837 and https://github.com/inorbithr/core/pull/837.

## Problem

Twice.
`;

test("heading ids are the console's", () => {
  assert.equal(headingId("The safety gate: corrections as checks"), "the-safety-gate-corrections-as-checks");
  assert.equal(headingId("`iohr runner` and [links]"), "iohr-runner-and-links");
  assert.equal(headingId("Ünïcode ✓"), "ncode");
  assert.equal(headingId("!!!"), "section");
  assert.ok(headingId("a ".repeat(300)).length <= 200);
});

test("sections follow headings, skip fences, and number repeats like the console", () => {
  const s = sections(DOC);
  assert.deepEqual(
    s.map((x) => x.anchor),
    ["", "what-the-owner-gets", "the-job-state-machine", "problem", "problem-1"],
  );
  assert.deepEqual(
    s.map((x) => x.level),
    [0, 2, 3, 2, 2],
  );
  assert.match(s[2].body, /## not a heading/);
  const { body } = splitFrontMatter(DOC);
  for (const x of s.slice(1)) assert.equal(body.slice(x.offset, x.offset + x.body.length), x.body, x.anchor);
});

test("a selection becomes a Decisions quote and is found again after edits", () => {
  const s = sections(DOC);
  const q = quoteInSection(DOC, s[3], "one chat on one Mac")!;
  assert.equal(q.exact, "one chat on one Mac");
  assert.ok(q.prefix!.endsWith("The coordinator is "));
  assert.ok(q.suffix!.startsWith(". See"));
  assert.equal(anchorAt(DOC, q.start!), "problem");
  const edited = DOC.replace("Intro line.", "A much longer introduction, written later, moves every offset.");
  const at = locateQuote(edited, q)!;
  assert.equal(splitFrontMatter(edited).body.slice(at.start, at.end), "one chat on one Mac");
  assert.equal(locateQuote(DOC.replace("one chat on one Mac", "a service"), q), null);
});

test("a passage with Markdown marks is still matched on its words", () => {
  const s = sections(DOC);
  const q = quoteInSection(DOC, s[1], "A coordinator that is always online")!;
  assert.ok(q);
  assert.match(q.exact, /always\*\* online/);
});

test("the surroundings pick the right one of two equal passages", () => {
  const text = "alpha beta gamma\n\ndelta beta epsilon\n";
  const second = quoteFor(text, text.indexOf("beta", 10), text.indexOf("beta", 10) + 4)!;
  const moved = "intro\n" + text;
  const at = locateQuote(moved, { ...second, start: undefined })!;
  assert.equal(moved.slice(at.start - 6, at.start), "delta ");
});

test("quotes are refused when empty or too long", () => {
  assert.equal(quoteFor("abc", 1, 1), null);
  assert.equal(quoteFor("x".repeat(2000), 0, 1500), null);
});

test("open threads are counted per heading; replies and resolved ones are not", () => {
  const m = countByAnchor([
    { anchor: "problem" },
    { anchor: "problem" },
    { anchor: "problem", parent_id: "c1" },
    { anchor: "", resolved: true },
    { anchor: "" },
  ]);
  assert.equal(m.get("problem"), 2);
  assert.equal(m.get(""), 1);
});

test("target keys tell an item, an element and a passage apart", () => {
  assert.equal(targetKey({ item_id: "itm_a" }), "itm_a");
  assert.equal(targetKey({ item_id: "itm_a", element: "envoy" }), "itm_a#envoy");
  assert.notEqual(
    targetKey({ item_id: "itm_a", element: "p", quote: { exact: "x", start: 1 } }),
    targetKey({ item_id: "itm_a", element: "p", quote: { exact: "x", start: 2 } }),
  );
});

test("PR links are found once each", () => {
  assert.deepEqual(prsIn(DOC), ["https://github.com/inorbithr/core/pull/837"]);
});

test("replacing a section keeps everything else byte for byte", () => {
  const next = replaceSection(DOC, "the-job-state-machine", "Twelve states.")!;
  assert.ok(next.includes("### The job state machine\n\nTwelve states.\n\n## Problem"));
  assert.ok(next.startsWith("---\ntitle: A test"));
  assert.equal(next.replace("\nTwelve states.\n", "\n```\n## not a heading\n```\nIntake to Done.\n"), DOC);
  assert.equal(replaceSection(DOC, "nope", "x"), null);
  const last = replaceSection(DOC, "problem-1", "Once.")!;
  assert.ok(last.endsWith("## Problem\n\nOnce.\n"));
});
