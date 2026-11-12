import { useEffect, useRef } from "react";
import { createHost, type Handlers, type Press } from "./shortcut-registry.ts";
import { BINDINGS } from "./shortcut-tables.ts";

// The web UI's one keydown listener. A page or area calls useShortcuts("<scope>", { "<binding id>": run }) with the
// ids its keys file declares (src/shortcuts/pages/<scope>.keys.json); the handlers are live while it is mounted.
// "global" is the frame's scope. Never add a keydown listener of your own for a shortcut: it would not be listed,
// and the test that finds two keys colliding would not see it.
const host = createHost(BINDINGS);
let attached = false;

function attach() {
  if (attached) return;
  attached = true;
  window.addEventListener("keydown", (e) => void host.press(e as Press, e.timeStamp), true);
}

export function useShortcuts(scope: string, handlers: Handlers) {
  const now = useRef(handlers);
  now.current = handlers;
  useEffect(() => {
    attach();
    return host.mount(scope, () => now.current);
  }, [scope]);
}
