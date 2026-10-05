// A client configured by `load` (the environment, the iohr config file, the iohr login),
// a middleware of your own, and the effective configuration with every value's source.
// Run after `iohr login`, or with INORBIT_TOKEN set.
import { ConfigError, type Middleware, Public } from "@inorbithr/sdk";

// Runs on every attempt, after `auth`: it sees the finished request.
const team: Middleware = {
  name: "team",
  handle(request, next) {
    request.headers.set("x-team", "payments");
    return next(request);
  },
};

let api: Public;
try {
  api = Public.load({ timeout: 10_000, pipeline: (p) => p.addPerRetry(team) });
} catch (e) {
  if (e instanceof ConfigError) {
    // Every problem, each with its setting and where the value came from.
    console.error(e.message);
    process.exit(1);
  }
  throw e;
}

const described = api.client.config().describe();
console.log(
  `credential from ${described.credential?.source}, pipeline ${described.pipeline.join(" > ")}`,
);

const { value: me, raw } = await api.me();
console.log(
  `subject ${me.subject}; request ${raw.requestId}; rate limit ${JSON.stringify(raw.rateLimit)}`,
);
