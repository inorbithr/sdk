// The SDK retries for you. The API answers 429 with Retry-After; the SDK waits that long
// and tries again, and the answer says how many attempts it took. Tune it with maxRetries.
import { Client, Public } from "@inorbithr/sdk";

const api = new Public(Client.load({ maxRetries: 3 }));
const { value: me, raw } = await api.me();
console.log(`${me.subject} after ${raw.attempts} attempts`);
