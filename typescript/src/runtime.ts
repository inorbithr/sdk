/**
 * The runtime alone, without the generated public surface: what a generated surface
 * imports. Programs import `@inorbithr/sdk`, which is this plus the public surface.
 *
 * @module
 */

export {
  AUDIENCE,
  CachedToken,
  type CachedTokenOptions,
  ChainedCredential,
  ClientCredentials,
  type ClientCredentialsOptions,
  CliToken,
  type CliTokenOptions,
  DEFAULT_TOKEN_URL,
  StaticToken,
  type Token,
  TokenFile,
  type TokenProvider,
} from "./auth.js";
export {
  type CallOptions,
  Client,
  type ClientOptions,
  DEFAULT_BASE_URL,
  DefaultCredential,
  type LoadClientOptions,
  loadConfig,
  MAX_BODY,
  type Method,
  type Operation,
  ResolvedConfig,
  type Response,
  type StreamOperation,
} from "./client.js";
export * as codegen from "./codegen.js";
export {
  ApiError,
  AuthError,
  CODES,
  type Code,
  ConfigError,
  type ConfigProblem,
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
export {
  type CallInfo,
  type Middleware,
  type Next,
  Pipeline,
  type SdkRequest,
  type SdkResponse,
  type Stage,
} from "./pipeline.js";
export type { Os } from "./platform.js";
export type { RateLimit, RateLimitPolicy } from "./ratelimit.js";
export {
  type CredentialSource,
  type DescribedSetting,
  type Description,
  type LoadOptions,
  parseDuration,
  type TriedSource,
} from "./settings.js";
export { MAX_EVENT, SOCKET_QUEUE, type StreamTransport } from "./stream.js";
export type {
  Logger,
  LogLevel,
  LogRecord,
  MeterProviderLike,
  OpenTelemetryApi,
  Redact,
  SpanContextLike,
  SpanLike,
  TracerLike,
  TracerProviderLike,
} from "./telemetry.js";
export { parseTimestamp } from "./timestamp.js";
export { SDK_VERSION } from "./version.js";
