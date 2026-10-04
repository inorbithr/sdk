// The account's events as they happen, printed for a minute or until Ctrl-C.
// Needs a token or key with events:read (INORBIT_TOKEN, or INORBIT_KEY_ID,
// INORBIT_KEY_SECRET and INORBIT_SCOPES="events:read"). The stream opens as server-sent
// events; a client built with `new Client({ ..., streams: "socket" })` carries every
// stream over one /v1/ws connection instead (Node, Deno, Bun).
import { ApiError, Public, TimeoutError } from "@inorbithr/sdk";

const api = Public.fromEnv();
const signal = AbortSignal.timeout(60_000);
try {
  for await (const event of api.events.streamEvents({}, { signal })) {
    console.log(`${event.occurred_at} ${event.type} ${event.id}`);
  }
} catch (e) {
  if (e instanceof ApiError && e.code === "forbidden") {
    console.error("the credential lacks events:read");
  } else if (e instanceof TimeoutError) {
    console.error("the stream went silent for 45 s; open it again");
  } else if (!signal.aborted) {
    throw e;
  }
}
