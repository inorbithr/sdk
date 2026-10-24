// The same event stream over one WebSocket (/v1/ws) instead of SSE: set `streams: "socket"`
// (Node, Deno and Bun; browsers use SSE). Every stream of the client shares the connection.
// Needs events:read.
import { Client, Public } from "@inorbithr/sdk";

const api = new Public(Client.load({ streams: "socket" }));
const signal = AbortSignal.timeout(30_000);
let n = 0;
for await (const event of api.events.streamEvents({}, { signal })) {
  n++;
  console.log(`${event.occurred_at} ${event.type} ${event.id}`);
}
console.log(`${n} events`);
