// The key rules moved from the Mac app's shortcuts.ts (its tests' cases kept), and the frame's own shortcut.
import assert from "node:assert/strict";
import { test } from "node:test";
import { accelOfEvent, canon, FRAME_SHORTCUTS, keysOf, matches, shortcutOf, symbols } from "../src/frame/shortcuts.ts";

const key = (
  k: string,
  o: Partial<{ metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean }> = {},
) => ({
  key: k,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...o,
});

test("the keys read as symbols", () => {
  assert.equal(symbols("Ctrl+Cmd+F"), "⌃⌘F");
  assert.equal(symbols("Alt+Shift+Cmd+S"), "⌥⇧⌘S");
  assert.equal(symbols("CmdOrCtrl+\\"), "⌘\\");
  assert.equal(symbols("CmdOrCtrl+,"), "⌘,");
  assert.equal(keysOf("sidebar"), "⌘\\");
  assert.equal(keysOf("nope"), "");
});

test("one spelling per accelerator, from a key event too", () => {
  assert.equal(canon("cmdorctrl+alt+p"), "Alt+Cmd+P");
  assert.equal(accelOfEvent({ ...key("π", { altKey: true, metaKey: true }), code: "KeyP" }), "Alt+Cmd+P");
  assert.equal(accelOfEvent(key("Meta", { metaKey: true })), "");
});

test("matching: Cmd and Ctrl are told apart, Ctrl stands in for Cmd off the Mac", () => {
  assert.equal(matches("Alt+Cmd+O", key("o", { metaKey: true })), false);
  assert.equal(matches("Alt+Cmd+P", { ...key("π", { altKey: true, metaKey: true }), code: "KeyP" }), true);
  assert.equal(matches("CmdOrCtrl+\\", key("\\", { ctrlKey: true })), true);
  assert.equal(matches("CmdOrCtrl+\\", key("\\", { ctrlKey: true, metaKey: true })), false);
});

test("the frame answers the Mac app's sidebar shortcut, and nothing else", () => {
  assert.deepEqual(FRAME_SHORTCUTS, [{ id: "sidebar", label: "Toggle sidebar", accel: "CmdOrCtrl+\\" }]);
  assert.equal(shortcutOf(key("\\", { metaKey: true }))?.id, "sidebar");
  assert.equal(shortcutOf(key("\\")), undefined);
  assert.equal(shortcutOf(key("\\", { metaKey: true, shiftKey: true })), undefined);
  assert.equal(shortcutOf(key("k", { metaKey: true })), undefined);
});
