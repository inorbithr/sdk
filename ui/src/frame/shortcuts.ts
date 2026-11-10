// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 InOrbit d.o.o.
//
// The key rules moved from iohr-mac src/lib/shortcuts.ts (origin/main d3ac8759) under ADR 0230: how an accelerator
// ("CmdOrCtrl+\\") is spelt one way, shown as Mac glyphs and matched against a key event. iohr-mac imports these
// rules from here (RFC 0217 C8) and keeps its own table, its rebinding and its native-menu delivery.
//
// The frame's own shortcuts live in one file, frame/shortcuts.json: id, label and accelerator. This module reads it,
// and so does iohr-mac, both its page (src/lib/shortcuts.ts) and its Rust native menu (keys.rs, by include_str!),
// which add only where the Mac menu bar places each row. A browser has no native menu, so a keydown listener on the
// window delivers them (the Mac app's fallback outside Tauri does the same).
import { useEffect, useRef } from "react";
import frameTable from "./shortcuts.json" with { type: "json" };

export type KeyLike = {
  key: string;
  code?: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
};

const SYMBOLS: Record<string, string> = {
  ctrl: "⌃",
  alt: "⌥",
  shift: "⇧",
  cmd: "⌘",
  cmdorctrl: "⌘",
  arrowleft: "←",
  arrowright: "→",
  arrowup: "↑",
  arrowdown: "↓",
  enter: "↩",
  backspace: "⌫",
};
const ORDER = ["ctrl", "alt", "shift", "cmd"];
const NAMES: Record<string, string> = {
  ctrl: "Ctrl",
  alt: "Alt",
  shift: "Shift",
  cmd: "Cmd",
  cmdorctrl: "Cmd",
  option: "Alt",
  control: "Ctrl",
  command: "Cmd",
};

const split = (accel: string) => accel.split(/\+(?=.)/);

/** One spelling per accelerator: modifiers in a fixed order, the key upper-cased. */
export function canon(accel: string): string {
  const parts = split(accel);
  const key = parts[parts.length - 1];
  const mods = parts
    .slice(0, -1)
    .map((p) => p.toLowerCase())
    .map((p) => (p === "cmdorctrl" ? "cmd" : NAMES[p] ? NAMES[p].toLowerCase() : p));
  const sorted = ORDER.filter((m) => mods.includes(m)).map((m) => NAMES[m]);
  return [...sorted, key.length === 1 ? key.toUpperCase() : key[0].toUpperCase() + key.slice(1)].join("+");
}

/** A key event as an accelerator string ("" while only modifiers are down). */
export function accelOfEvent(e: KeyLike): string {
  if (["Meta", "Control", "Alt", "Shift"].includes(e.key)) return "";
  const k = e.code?.startsWith("Key") ? e.code.slice(3) : e.code?.startsWith("Digit") ? e.code.slice(5) : e.key;
  const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Cmd"].filter(Boolean);
  return [...mods, k.length === 1 ? k.toUpperCase() : k].join("+");
}

/** "Ctrl+Cmd+F" → "⌃⌘F". */
export const symbols = (accel: string) =>
  split(accel)
    .map((p) => SYMBOLS[p.toLowerCase()] ?? p.toUpperCase())
    .join("");

/** Whether a key event is this accelerator (⌘ as Cmd; Ctrl stands in for Cmd off the Mac). */
export function matches(accel: string, e: KeyLike): boolean {
  const parts = split(accel);
  const key = parts[parts.length - 1].toLowerCase();
  const mods = parts.slice(0, -1).map((p) => p.toLowerCase());
  const wantCtrl = mods.includes("ctrl");
  const wantCmd = mods.includes("cmd") || mods.includes("cmdorctrl");
  const cmdOrCtrl = mods.includes("cmdorctrl") && !wantCtrl;
  const hasCmd = cmdOrCtrl ? e.metaKey !== e.ctrlKey : e.metaKey;
  const hasCtrl = cmdOrCtrl ? false : e.ctrlKey;
  // ⌥ turns letters into other characters (⌥P is π), so a letter also matches by its physical key.
  const keyOk =
    e.key.toLowerCase() === key ||
    (key.length === 1 && (e.code === `Key${key.toUpperCase()}` || e.code === `Digit${key}`));
  return (
    keyOk &&
    wantCmd === hasCmd &&
    wantCtrl === hasCtrl &&
    mods.includes("alt") === e.altKey &&
    mods.includes("shift") === e.shiftKey
  );
}

// ---------------------------------------------------------------- the frame's shortcuts

export type FrameShortcut = { id: string; label: string; accel: string };

/** The shortcuts the frame answers, from frame/shortcuts.json: the one table of them. */
export const FRAME_SHORTCUTS: FrameShortcut[] = frameTable;

/** The frame shortcut a key event is, if any. */
export const shortcutOf = (e: KeyLike) => FRAME_SHORTCUTS.find((s) => matches(s.accel, e));

/** The keys to show for a frame shortcut id; "" for an unknown id. */
export const keysOf = (id: string) => {
  const s = FRAME_SHORTCUTS.find((x) => x.id === id);
  return s ? symbols(s.accel) : "";
};

/** Runs the handler of each frame shortcut pressed anywhere in the page; ids without a handler pass through. */
export function useFrameShortcuts(handlers: Partial<Record<string, () => void>>) {
  const now = useRef(handlers);
  now.current = handlers;
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      const s = shortcutOf(e);
      const run = s ? now.current[s.id] : undefined;
      if (!run) return;
      e.preventDefault();
      run();
    };
    window.addEventListener("keydown", on, true);
    return () => window.removeEventListener("keydown", on, true);
  }, []);
}
