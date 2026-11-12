// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 InOrbit d.o.o.
//
// Moved from iohr-mac src/components/ui.tsx (origin/main d3ac8759) under ADR 0230: the shell frame layer
// lives here under Apache-2.0, and iohr-mac imports it (RFC 0217 C8 deletes its copy).

/** Joins the class names that are set, so a condition can switch a class off with `false`. */
export const cn = (...c: (string | false | null | undefined)[]) => c.filter(Boolean).join(" ");
