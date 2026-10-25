// A server-sent-events stream: the account's events, as they happen. Each event is typed;
// the loop ends when the server ends the stream or the signal aborts.
// Needs events:read (INORBIT_TOKEN, or a key with INORBIT_SCOPES="events:read").
import { Public } from "@inorbithr/sdk";

const api = Public.fromEnv(); // streams open as SSE by default
const signal = AbortSignal.timeout(30_000);
let n = 0;
for await (const event of api.events.streamEvents({}, { signal })) {
  n++;
  console.log(`${event.occurred_at} ${event.type} ${event.id}`);
}
console.log(`${n} events`);
