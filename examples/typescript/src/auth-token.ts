// Auth with a token you already have: an API token from the console or `iohr token create`.
// The SDK sends it as a Bearer on every call and never exchanges it.
import { Client, Public } from "@inorbithr/sdk";

const api = new Public(
  new Client({
    token: process.env.INORBIT_TOKEN,
    baseUrl: process.env.INORBIT_BASE_URL,
  }),
);
const { value: me } = await api.me();
console.log(`${me.subject} signed in with a token`);
