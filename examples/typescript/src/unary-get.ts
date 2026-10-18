// One unary GET: who the API thinks you are. The answer carries the typed value and the
// raw response (request id, attempts, rate limit).
// Run with INORBIT_TOKEN set (or INORBIT_KEY_ID, INORBIT_KEY_SECRET and INORBIT_SCOPES).
import { Public } from "@inorbithr/sdk";

const api = Public.fromEnv();
const { value: me, raw } = await api.me();
console.log(`subject ${me.subject}, scopes ${me.scopes.join(" ")}, request ${raw.requestId}`);
