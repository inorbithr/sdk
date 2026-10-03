/**
 * The InOrbit API for TypeScript and JavaScript: the runtime, and the public surface
 * generated from the API's public document. A surface cut to your own credentials comes
 * from `iohr sdk generate --lang typescript`.
 *
 * ```ts
 * import { Public } from "@inorbithr/sdk";
 *
 * const api = Public.fromEnv();
 * const { value: me } = await api.me();
 * ```
 *
 * @module
 */

export * from "./generated/index.js";
export * from "./runtime.js";
