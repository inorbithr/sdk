// Pictures on the canvas (src/canvas/image.ts): the sizes a pasted image is stored and shown at.
import assert from "node:assert/strict";
import { test } from "node:test";
import { fitSize, imageFiles, MAX_SIDE, shownSize } from "../src/canvas/image.ts";

test("an image over 2400 px on its long side is scaled down, the shape kept; a smaller one is left alone", () => {
  assert.deepEqual(fitSize(4800, 2700), { w: MAX_SIDE, h: 1350 });
  assert.deepEqual(fitSize(1000, 5000), { w: 480, h: MAX_SIDE });
  assert.deepEqual(fitSize(1600, 900), { w: 1600, h: 900 });
  assert.deepEqual(fitSize(0, 0), { w: 1, h: 1 });
});

test("a new image is shown at most 640 by 480, never enlarged", () => {
  assert.deepEqual(shownSize(2400, 1350), { w: 640, h: 360 });
  assert.deepEqual(shownSize(400, 300), { w: 400, h: 300 });
  assert.deepEqual(shownSize(600, 1200), { w: 240, h: 480 });
});

test("only PNG, WebP and JPEG files are taken", () => {
  const f = (type: string) => ({ type }) as File;
  assert.equal(
    imageFiles([f("image/png"), f("image/svg+xml"), f("text/plain"), f("image/webp"), f("image/jpeg"), f("image/gif")])
      .length,
    3,
  );
  assert.deepEqual(imageFiles(null), []);
});
