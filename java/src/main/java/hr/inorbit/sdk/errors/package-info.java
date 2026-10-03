/**
 * Every way a call fails, as one sealed tree under {@link hr.inorbit.sdk.errors.InOrbitException}:
 * the API's answer ({@link hr.inorbit.sdk.errors.ApiException}, with its {@link
 * hr.inorbit.sdk.errors.Code} and {@link hr.inorbit.sdk.errors.Detail}s), and connection, timeout,
 * auth, configuration, size and decoding failures. All are unchecked.
 */
package hr.inorbit.sdk.errors;
