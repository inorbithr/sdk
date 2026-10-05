/**
 * The middleware pipeline (docs/config.md section 7): every call goes through an ordered list of
 * named {@link hr.inorbit.sdk.middleware.Middleware}s, the per-call stage once and the per-retry
 * stage once per attempt, then the transport. Edit it by name with {@link
 * hr.inorbit.sdk.middleware.Pipeline} when the client is built.
 */
package hr.inorbit.sdk.middleware;
