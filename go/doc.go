// Package inorbit is the runtime of the InOrbit API SDK for Go: the client, its
// credentials, errors, retries and hooks, and the one request path every operation
// goes through.
//
// The operations themselves are generated. The public surface, the operations an API
// key or token may call, is in the public package of this module. A surface cut to
// your own credentials, with one package per profile, comes from
// `iohr sdk generate --lang go`:
//
//	ci, err := acmeci.FromEnv() // INORBIT_ACME_CI_TOKEN, or _KEY_ID, _KEY_SECRET and _SCOPES
//	if err != nil {
//		return err
//	}
//	digests, err := ci.Radar().ListDigests(ctx, nil)
//
// Every call takes a context first; errors are *APIError, *ConnectionError,
// *TimeoutError, *AuthError, *ConfigError, *TooLargeError or *DecodeError, told apart
// with errors.As. Credentials never print: their String and GoString redact the secret.
package inorbit
