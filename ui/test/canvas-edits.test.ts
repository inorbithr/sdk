// Edits the coordinator proposes on a canvas (src/canvas/edits.ts). Run with `pnpm test`.
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  applyEdits,
  askPrompt,
  commentPrompt,
  describe,
  FENCE,
  proposalsIn,
  suggestionBody,
} from "../src/canvas/edits.ts";
import { fixtureCanvas as seedCanvas } from "./canvas-fixture.ts"; // edits on a diagram with groups

const reply = `Here is what I would change.

\`\`\`${FENCE}
{"summary": "Name the takeover", "edits": [
  {"op": "add_node", "item_id": "itm_diagram-architecture", "node": {"label": "Takeover sweep", "kind": "function", "group": "coordinator"}, "edges": [{"source": "leases", "target": "takeover-sweep", "label": "90 s silence"}]},
  {"op": "update_node", "item_id": "itm_diagram-architecture", "node_id": "envoy", "detail": "verifies every token, rate limits"},
  {"op": "launch_rockets"}
]}
\`\`\`

And a note:

\`\`\`${FENCE}
[{"op": "add_note", "text": "Ask Legal before slice 3.", "near": "itm_note-status"}]
\`\`\`

\`\`\`${FENCE}
{not json
\`\`\`
`;

test("proposals are read from fenced blocks; broken ones and unknown ops are skipped", () => {
  const ps = proposalsIn(reply, "msg1:");
  assert.equal(ps.length, 2);
  assert.equal(ps[0].summary, "Name the takeover");
  assert.equal(ps[0].edits.length, 2);
  assert.match(ps[1].summary, /Add a note/);
  assert.ok(ps[0].key.startsWith("msg1:0:"));
  assert.deepEqual(
    proposalsIn(reply, "msg1:").map((p) => p.key),
    ps.map((p) => p.key),
    "keys are stable",
  );
});

test("applying adds the node with an id from its label, and the edge", () => {
  const c = seedCanvas();
  const [p] = proposalsIn(reply);
  const r = applyEdits(c, p.edits);
  assert.equal(r.skipped.length, 0);
  const d = r.canvas.items.find((i) => i.item_id === "itm_diagram-architecture")!;
  assert.equal(d.type, "diagram");
  if (d.type !== "diagram") return;
  const node = d.model.nodes.find((n) => n.id === "takeover-sweep")!;
  assert.equal(node.group, "coordinator");
  assert.equal(node.x, undefined, "the frame lays it out");
  assert.ok(d.model.edges.some((e) => e.source === "leases" && e.target === "takeover-sweep"));
  assert.equal(d.model.nodes.find((n) => n.id === "envoy")!.detail, "verifies every token, rate limits");
  assert.notEqual(c, r.canvas, "the input is never changed");
  assert.ok(
    !(
      c.items.find((i) => i.item_id === "itm_diagram-architecture") as { model: { nodes: { id: string }[] } }
    ).model.nodes.some((n) => n.id === "takeover-sweep"),
  );
});

test("edits that can't apply are skipped with a reason; section rewrites are handed back", () => {
  const c = seedCanvas();
  const r = applyEdits(c, [
    { op: "remove_node", item_id: "itm_diagram-architecture", node_id: "nope" },
    { op: "add_edge", item_id: "itm_note-idea", source: "a", target: "b" },
    { op: "replace_section", item_id: "itm_doc-rfc-0120", anchor: "problem", text: "Shorter." },
    { op: "rename", item_id: "itm_note-idea", title: "The idea, in one line" },
    { op: "add_link", from: "itm_note-idea", to: "itm_doc-adr-0075", label: "decided in" },
  ]);
  assert.equal(r.skipped.length, 2);
  assert.match(r.skipped[0], /no node nope/);
  assert.match(r.skipped[1], /no diagram/);
  assert.equal(r.sections.length, 1);
  assert.equal(r.canvas.items.find((i) => i.item_id === "itm_note-idea")!.title, "The idea, in one line");
  assert.equal(r.canvas.links.length, c.links.length + 1);
});

test("removing a node removes its edges", () => {
  const c = seedCanvas();
  const r = applyEdits(c, [{ op: "remove_node", item_id: "itm_diagram-architecture", node_id: "envoy" }]);
  const d = r.canvas.items.find((i) => i.item_id === "itm_diagram-architecture");
  assert.ok(d?.type === "diagram" && !d.model.edges.some((e) => e.source === "envoy" || e.target === "envoy"));
});

test("the ask carries the item's id, title and text, and how to propose an edit", () => {
  const t = askPrompt({
    canvas: "Coordinator as a Service",
    canvasId: "cnv_x",
    what: "diagram node",
    itemId: "itm_diagram-architecture",
    element: "envoy",
    title: "Envoy",
    text: "verifies every token",
    question: "Should it rate limit?",
  });
  assert.match(t, /itm_diagram-architecture/);
  assert.match(t, /element envoy/);
  assert.match(t, /verifies every token/);
  assert.match(t, /Should it rate limit\?/);
  assert.match(t, new RegExp(FENCE));
  assert.ok(!t.includes("'"), "no single quote, so it can be typed into a shell-quoted line");
  const long = askPrompt({
    canvas: "c",
    canvasId: "cnv_x",
    what: "document",
    itemId: "itm_a",
    title: "t",
    text: "x".repeat(5000),
    question: "",
  });
  assert.match(long, /cut at 2,400/);
  assert.match(describe({ op: "replace_section", item_id: "i", anchor: "a", text: "" }), /new version/);
});

test("a rewrite is posted as a suggestion, never as the document's text (ADR 0077)", () => {
  const b = suggestionBody("goals", 7, "  New goals.\n");
  assert.match(b, /^\*\*Suggested change to #goals\*\* \(on version 7,/);
  assert.match(b, /the system and then a person/);
  assert.ok(b.endsWith("New goals."));
});

test("update_text rewrites a note or a frame and is skipped on a diagram", () => {
  const c = seedCanvas();
  const note = c.items.find((i) => i.type === "note");
  const frame = c.items.find((i) => i.type === "frame");
  assert.ok(note && frame);
  const r = applyEdits(c, [
    { op: "update_text", item_id: note.item_id, text: "Rewritten note." },
    { op: "update_text", item_id: frame.item_id, text: "Rewritten frame." },
    { op: "update_text", item_id: "itm_diagram-architecture", text: "no" },
  ]);
  const get = (id: string) => r.canvas.items.find((i) => i.item_id === id) as { text?: string };
  assert.equal(get(note.item_id).text, "Rewritten note.");
  assert.equal(get(frame.item_id).text, "Rewritten frame.");
  assert.equal(r.done.length, 2);
  assert.equal(r.skipped.length, 1);
});

test("an answer to a comment names its thread; a reply alone is still an answer", () => {
  const text = `\`\`\`${FENCE}
{"thread": "thr_1", "reply": "Done, shortened it.", "summary": "Shorten the note", "edits": [{"op": "update_text", "item_id": "itm_a", "text": "Short."}]}
\`\`\`

\`\`\`${FENCE}
{"thread": "thr_2", "reply": "That part is right as it is.", "edits": []}
\`\`\``;
  const ps = proposalsIn(text, "m:");
  assert.equal(ps.length, 2);
  assert.equal(ps[0].thread, "thr_1");
  assert.equal(ps[0].reply, "Done, shortened it.");
  assert.equal(ps[0].edits.length, 1);
  assert.equal(ps[1].thread, "thr_2");
  assert.equal(ps[1].edits.length, 0);
  assert.equal(
    proposalsIn(`\`\`\`${FENCE}\n{"edits": []}\n\`\`\``).length,
    0,
    "an empty block without a thread is nothing",
  );
});

test("the comment prompt carries the thread, the part and the fence", () => {
  const p = commentPrompt({
    canvas: "Board",
    canvasId: "cnv_x",
    threadId: "thr_9",
    what: "note",
    itemId: "itm_a",
    title: "Status",
    text: "Old text",
    conversation: [{ author: "Nevio", body: "Make it shorter" }],
  });
  assert.match(p, /thr_9/);
  assert.match(p, /itm_a/);
  assert.match(p, /Old text/);
  assert.match(p, /Make it shorter/);
  assert.ok(p.includes(FENCE));
});
