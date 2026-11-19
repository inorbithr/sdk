#!/usr/bin/env node
import { run } from "./cli.ts";

// Passes the real streams and exits with the result of run().
process.exitCode = run(process.argv.slice(2), {
  out: (s) => process.stdout.write(s),
  err: (s) => process.stderr.write(s),
});
