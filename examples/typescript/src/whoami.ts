// Who the API thinks you are, and the latest radar digests, with the public surface.
// Run with INORBIT_TOKEN set (an API token from the console or `iohr token create`).
import { ApiError, Public } from "@inorbithr/sdk";

const api = Public.fromEnv();
try {
  const { value: me } = await api.me();
  console.log(`subject ${me.subject}, scopes ${me.scopes.join(" ")}`);
  const { value: page } = await api.radar.listDigests({ limit: 3 });
  for (const d of page.digests) {
    console.log(`${d.week} ${d.language}: ${d.summary}`);
  }
} catch (e) {
  if (e instanceof ApiError && e.code === "forbidden") {
    console.error("the token lacks a scope this example needs (identity:read, radar:read)");
  } else {
    throw e;
  }
}
