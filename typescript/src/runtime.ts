/**
 * The runtime alone, without the generated public surface: what a generated surface
 * imports. Programs import `@inorbithr/sdk`, which is this plus the public surface.
 *
 * @module
 */

export {
  AUDIENCE,
  ClientCredentials,
  type ClientCredentialsOptions,
  DEFAULT_TOKEN_URL,
  StaticToken,
  type Token,
  type TokenProvider,
} from "./auth.js";
export {
  type CallOptions,
  Client,
  type ClientOptions,
  DEFAULT_BASE_URL,
  MAX_BODY,
  type Method,
  type Operation,
  type Response,
} from "./client.js";
export * as codegen from "./codegen.js";
export {
  ApiError,
  AuthError,
  CODES,
  type Code,
  ConfigError,
  ConnectionError,
  DecodeError,
  type Detail,
  InOrbitError,
  type KnownCode,
  RawResponse,
  TimeoutError,
  TooLargeError,
} from "./errors.js";
export type { Attempt, Hook } from "./hooks.js";
export { formatInt64, type Int64, parseInt64 } from "./int64.js";
export { SDK_VERSION } from "./version.js";
