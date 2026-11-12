// Reads HOST and PORT from the environment, serves, and stops cleanly on SIGINT or SIGTERM.

import { Logger } from "./obs.ts";
import { Service, makeServer } from "./server.ts";

export function loadConfig(env: Record<string, string | undefined>): { host: string; port: number } {
  const raw = env["PORT"] || "8080";
  if (!/^\d+$/.test(raw) || Number(raw) > 65535) throw new Error("PORT must be a number from 0 to 65535");
  return { host: env["HOST"] || "127.0.0.1", port: Number(raw) };
}

function main(): void {
  const log = new Logger(process.stdout);
  let cfg;
  try {
    cfg = loadConfig(process.env);
  } catch (e) {
    log.error("bad configuration", { error: (e as Error).message });
    process.exit(2);
  }
  const svc = new Service(log);
  const server = makeServer(svc);
  server.listen(cfg.port, cfg.host, () => {
    svc.ready = true;
    const a = server.address();
    log.info("listening", { addr: typeof a === "object" && a ? `${a.address}:${a.port}` : String(a) });
  });
  const stop = () => {
    svc.ready = false;
    log.info("shutting down");
    server.close(() => process.exit(0));
    server.closeIdleConnections();
    setTimeout(() => process.exit(1), 10_000).unref();
  };
  process.on("SIGINT", stop);
  process.on("SIGTERM", stop);
}

// Run as a program, not when a test imports loadConfig.
if (import.meta.url === new URL(process.argv[1] ?? "", "file://").href) main();
