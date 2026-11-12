// A small canvas for the tests: a diagram with groups, notes, a frame, two documents and a link.
// Built by hand so the tests do not depend on the Mac app's demo seeds.
import { type Canvas, type DiagramModel, emptyCanvas } from "../src/canvas/schema.ts";

export const ARCH: DiagramModel = {
  version: 3,
  direction: "LR",
  nodes: [
    { id: "envoy", label: "Envoy", kind: "edge", detail: "verifies every token" },
    { id: "leases", label: "Leases", kind: "part", group: "coordinator" },
    { id: "driver", label: "Driver", kind: "part", group: "coordinator" },
  ],
  groups: [{ id: "coordinator", label: "Coordinator", kind: "service" }],
  edges: [
    { id: "envoy--leases", source: "envoy", target: "leases" },
    { id: "leases--driver", source: "leases", target: "driver" },
  ],
};

export function fixtureCanvas(): Canvas {
  const c = emptyCanvas("Fixture", "2026-11-10T10:00:00.000Z");
  c.canvas_id = "cnv_fixture";
  c.items = [
    {
      item_id: "itm_diagram-architecture",
      type: "diagram",
      title: "Architecture",
      x: 0,
      y: 0,
      w: 600,
      h: 400,
      model: structuredClone(ARCH),
    },
    {
      item_id: "itm_note-idea",
      type: "note",
      title: "The idea",
      text: "Edits are proposed, never applied.",
      tone: "amber",
      x: 700,
      y: 0,
      w: 240,
      h: 160,
    },
    { item_id: "itm_note-status", type: "note", title: "Status", text: "Slice 1.", x: 700, y: 200, w: 240, h: 160 },
    {
      item_id: "itm_frame-plan",
      type: "frame",
      title: "Plan",
      text: "What ships when",
      x: -40,
      y: -40,
      w: 1000,
      h: 520,
    },
    {
      item_id: "itm_doc-rfc-0120",
      type: "document",
      title: "RFC 0120",
      x: 0,
      y: 500,
      w: 360,
      h: 240,
      document: { space_id: "lab_x", document_id: "ldoc_x", kind: "rfc", display_number: "0120" },
    },
    {
      item_id: "itm_doc-adr-0075",
      type: "document",
      title: "ADR 0075",
      x: 400,
      y: 500,
      w: 360,
      h: 240,
      document: { space_id: "lab_x", document_id: "ldoc_y", kind: "adr", display_number: "0075" },
    },
  ];
  c.links = [
    { link_id: "lnk_1", from: { item_id: "itm_note-idea" }, to: { item_id: "itm_doc-rfc-0120" }, kind: "context" },
  ];
  return c;
}
