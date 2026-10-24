// Configuration by `load`: code, the environment, the iohr config file and the iohr login,
// in that order. describe() says where each value came from, without printing a secret.
import { ConfigError, Public } from "@inorbithr/sdk";

try {
  const api = Public.load({ timeout: 10_000 });
  const described = api.client.config().describe();
  console.log(
    `credential from ${described.credential?.source}, pipeline ${described.pipeline.join(" > ")}`,
  );
  const { value: me } = await api.me();
  console.log(`${me.subject}, a ${me.kind}`);
} catch (e) {
  if (e instanceof ConfigError) {
    console.error(e.message); // every problem, with its setting
    process.exit(1);
  }
  throw e;
}
