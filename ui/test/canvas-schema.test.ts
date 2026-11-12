// The canvas file (src/canvas/schema.ts). Run with `pnpm test`.
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  bounds,
  type Canvas,
  elementId,
  emptyCanvas,
  ID_RE,
  MAX_IMAGE_SRC,
  newId,
  parseCanvas,
  removeItems,
  serialize,
  VERSION,
} from "../src/canvas/schema.ts";
import { fixtureCanvas as seedCanvas } from "./canvas-fixture.ts";

test("a canvas survives a round trip unchanged", () => {
  const c = seedCanvas();
  const p = parseCanvas(serialize(c));
  assert.ok(p.ok);
  assert.deepEqual(p.canvas, c);
  assert.equal(p.migrated, false);
  assert.deepEqual(p.dropped, []);
});

test("ids are prefixed, ordered by time and unique", () => {
  const a = newId("itm", 1_700_000_000_000);
  const b = newId("itm", 1_700_000_000_000);
  const later = newId("itm", 1_700_000_000_001);
  assert.match(a, ID_RE);
  assert.notEqual(a, b);
  assert.ok(a.slice(0, 13) <= later.slice(0, 13));
  const many = new Set(Array.from({ length: 2000 }, () => newId("lnk")));
  assert.equal(many.size, 2000);
});

test("element ids look like heading anchors and never collide", () => {
  assert.equal(elementId("iohr runner on the Mac"), "iohr-runner-on-the-mac");
  assert.equal(elementId("  ***  "), "node");
  const taken = new Set(["envoy"]);
  assert.equal(elementId("Envoy", taken), "envoy-2");
});

test("what it can't read, it says, and never throws", () => {
  for (const [text, why] of [
    ["not json", /isn't JSON/],
    ["[]", /no object/],
    [JSON.stringify({ schema: "something.else" }), /schema is something.else/],
    [JSON.stringify({ schema: "iohr.canvas", version: VERSION + 1, canvas_id: "cnv_x" }), /newer InOrbit/],
    [JSON.stringify({ schema: "iohr.canvas", version: 1, canvas_id: "bad id" }), /canvas_id/],
  ] as const) {
    const p = parseCanvas(text);
    assert.equal(p.ok, false);
    if (!p.ok) assert.match(p.error, why);
  }
});

test("bad items are dropped and named; links to them go too", () => {
  const raw = {
    schema: "iohr.canvas",
    version: 1,
    canvas_id: "cnv_t",
    title: "t",
    items: [
      { item_id: "itm_a", type: "note", text: "hi", x: 0, y: 0, w: 100, h: 100 },
      { item_id: "itm_a", type: "note", text: "duplicate", x: 0, y: 0, w: 100, h: 100 },
      { item_id: "itm_b", type: "spaceship" },
      { item_id: "itm_c", type: "document", document: {} },
      { item_id: "itm_d", type: "frame", x: "nope", w: -5 },
    ],
    links: [
      { link_id: "lnk_1", from: { item_id: "itm_a" }, to: { item_id: "itm_d" }, kind: "context" },
      { link_id: "lnk_2", from: { item_id: "itm_a" }, to: { item_id: "itm_b" }, kind: "context" },
    ],
  };
  const p = parseCanvas(JSON.stringify(raw));
  assert.ok(p.ok);
  assert.deepEqual(
    p.canvas.items.map((i) => i.item_id),
    ["itm_a", "itm_d"],
  );
  assert.deepEqual(p.dropped, ["itm_a", "itm_b", "itm_c"]);
  assert.deepEqual(
    p.canvas.links.map((l) => l.link_id),
    ["lnk_1"],
  );
  const d = p.canvas.items[1];
  assert.equal(d.x, 0);
  assert.ok(d.w >= 40, "sizes are clamped to something drawable");
});

test("version 0 files are migrated", () => {
  const p = parseCanvas(
    JSON.stringify({
      id: "cnv_old",
      title: "Old",
      items: [{ id: "itm_n", kind: "note", text: "x", x: 1, y: 2, w: 100, h: 50 }],
    }),
  );
  assert.ok(p.ok);
  assert.equal(p.migrated, true);
  assert.equal(p.canvas.canvas_id, "cnv_old");
  assert.equal(p.canvas.items[0].type, "note");
  assert.deepEqual(p.canvas.threads, []);
});

test("diagram models drop dangling edges and unknown groups, and keep extra positions", () => {
  const p = parseCanvas(
    JSON.stringify({
      schema: "iohr.canvas",
      version: 1,
      canvas_id: "cnv_d",
      items: [
        {
          item_id: "itm_g",
          type: "diagram",
          x: 0,
          y: 0,
          w: 400,
          h: 300,
          model: {
            version: 3,
            nodes: [
              { id: "a", label: "A", kind: "rpc", x: 10, y: 20, pinned: true, group: "nope" },
              { id: "b", label: "B", kind: "weird" },
            ],
            edges: [
              { source: "a", target: "b" },
              { source: "a", target: "zzz" },
              { source: "a", target: "b" },
            ],
          },
        },
      ],
    }),
  );
  assert.ok(p.ok);
  const d = p.canvas.items[0];
  assert.equal(d.type, "diagram");
  if (d.type !== "diagram") return;
  assert.equal(d.model.version, 3);
  assert.equal(d.model.nodes[0].group, undefined);
  assert.equal(d.model.nodes[1].kind, "service");
  assert.deepEqual(
    d.model.edges.map((e) => e.id),
    ["a--b", "a--b-2"],
  );
  assert.equal(d.model.nodes[0].pinned, true);
});

test("removing an item removes its links and its threads", () => {
  const c: Canvas = seedCanvas();
  const withThread = {
    ...c,
    threads: [
      {
        thread_id: "thr_1",
        target: { item_id: "itm_doc-rfc-0120", element: "the-api" },
        comments: [{ comment_id: "cmt_1", author: "nevio", body: "x", created_at: "" }],
      },
    ],
  };
  const next = removeItems(withThread, ["itm_doc-rfc-0120"]);
  assert.ok(!next.items.some((i) => i.item_id === "itm_doc-rfc-0120"));
  assert.ok(!next.links.some((l) => l.from.item_id === "itm_doc-rfc-0120" || l.to.item_id === "itm_doc-rfc-0120"));
  assert.equal(next.threads.length, 0);
});

test("empty canvases and bounds", () => {
  const c = emptyCanvas("New");
  assert.match(c.canvas_id, /^cnv_/);
  assert.equal(bounds([]), null);
  assert.deepEqual(
    bounds([
      { x: 0, y: 0, w: 10, h: 10 },
      { x: -5, y: 20, w: 5, h: 5 },
    ]),
    { x: -5, y: 0, w: 15, h: 25 },
  );
});

// A 1x1 WebP, base64: the smallest picture the canvas keeps.
const PIC = "data:image/webp;base64,UklGRhoAAABXRUJQVlA4TA0AAAAvAAAAEAcQERGIiP4HAA==";

test("an image survives a round trip: parse, then serialize, keeps it", () => {
  const raw = {
    schema: "iohr.canvas",
    version: 3,
    canvas_id: "cnv_img",
    title: "t",
    items: [
      {
        item_id: "itm_pic",
        type: "image",
        title: "Terminal",
        src: PIC,
        alt: "today's terminal",
        natural_w: 1600,
        natural_h: 900,
        x: 10,
        y: 20,
        w: 640,
        h: 360,
      },
    ],
    links: [],
  };
  const a = parseCanvas(JSON.stringify(raw));
  assert.ok(a.ok);
  assert.deepEqual(a.dropped, []);
  const img = a.canvas.items[0];
  assert.equal(img.type, "image");
  assert.equal(img.type === "image" && img.src, PIC);
  const b = parseCanvas(serialize(a.canvas));
  assert.ok(b.ok);
  assert.deepEqual(b.canvas, a.canvas);
});

test("an image with a bad src is dropped and named; a link to it goes too", () => {
  const item = (id: string, src: unknown) => ({ item_id: id, type: "image", src, x: 0, y: 0, w: 100, h: 100 });
  const raw = {
    schema: "iohr.canvas",
    version: 3,
    canvas_id: "cnv_img",
    title: "t",
    items: [
      item("itm_ok", PIC),
      item("itm_http", "https://example.com/a.png"),
      item("itm_svg", "data:image/svg+xml;base64,PHN2Zz48L3N2Zz4="),
      item("itm_js", "javascript:alert(1)"),
      item("itm_plain", "data:image/png,not-base64"),
      item("itm_none", undefined),
      item("itm_big", "data:image/png;base64," + "A".repeat(MAX_IMAGE_SRC)),
    ],
    links: [{ link_id: "lnk_1", from: { item_id: "itm_ok" }, to: { item_id: "itm_http" }, kind: "context" }],
  };
  const p = parseCanvas(JSON.stringify(raw));
  assert.ok(p.ok);
  assert.deepEqual(
    p.canvas.items.map((i) => i.item_id),
    ["itm_ok"],
  );
  assert.deepEqual(p.dropped, ["itm_http", "itm_svg", "itm_js", "itm_plain", "itm_none", "itm_big"]);
  assert.deepEqual(p.canvas.links, []);
});
