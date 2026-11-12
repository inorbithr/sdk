import assert from "node:assert/strict";
import { test } from "node:test";
import { asView, sensorsRoute, viewFrom, viewHref } from "../src/lib/sensors-view.ts";

test("the view comes from the address, then what was kept, then the orbit", () => {
  assert.equal(viewFrom("?view=control", "orbit"), "control");
  assert.equal(viewFrom("", "control"), "control");
  assert.equal(viewFrom("?view=sessions", null), "orbit");
  assert.equal(viewFrom("", "nonsense"), "orbit");
});

test("only the two views exist, and the route is /sensors", () => {
  assert.equal(asView("sessions"), null);
  assert.equal(sensorsRoute("/sensors"), true);
  assert.equal(sensorsRoute("/sensors/"), true);
  assert.equal(sensorsRoute("/sensors/x"), false);
  assert.equal(viewHref("orbit"), "/sensors");
  assert.equal(viewHref("control"), "/sensors?view=control");
});
