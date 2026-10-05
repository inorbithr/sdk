# Changelog

## [0.2.2](https://github.com/inorbithr/sdk/compare/rust/v0.2.1...rust/v0.2.2) (2026-10-05)


### Features

* **cli:** iohr sdk config resolves through the Rust SDK's loader, and the Rust target emits the idempotency-key mark and the path template ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **conformance:** replay matchers, TLS, mTLS and proxy listeners, fake iohr (M6) ([#113](https://github.com/inorbithr/sdk/issues/113)) ([fe098b6](https://github.com/inorbithr/sdk/commit/fe098b64bbfec4980ec7d1f3f2efe925873498b1))
* **rust:** calls have a total timeout of 120 s by default (total_timeout), and a Retry-After longer than retry_after_max ends the call ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** Client::load with one precedence (code, environment, the iohr config file, defaults), describe() and ConfigError problems ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** proxy, CA bundle, mTLS, pinning and connect timeout through the SDK's own rules, or a caller's reqwest::Client ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** the credential chain (env, token and key secret files, the iohr login) with TokenFile, CliToken, CachedToken, ChainedCredential and DefaultCredential ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** the named middleware pipeline with its built-ins, tracing logging, OpenTelemetry behind the otel feature, rate limits and the retry budget ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))


### Bug fixes

* **rust:** the socket task ends when the client is dropped ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))


### Dependencies

* **rust:** tokio-tungstenite 0.30, pkcs8 0.11 and fresh lockfiles ([#135](https://github.com/inorbithr/sdk/issues/135)) ([d13b761](https://github.com/inorbithr/sdk/commit/d13b761ad7b96924c1d14bea08ecd3cd3517aace))

## [0.2.1](https://github.com/inorbithr/sdk/compare/rust/v0.2.0...rust/v0.2.1) (2026-10-04)


### Features

* streams in all six languages, over server-sent events or the /v1/ws socket (M5) ([#96](https://github.com/inorbithr/sdk/issues/96)) ([7ff73fe](https://github.com/inorbithr/sdk/commit/7ff73fe986f4706299d188ef9a4d5273dc0ca7ee))

## [0.2.0](https://github.com/inorbithr/sdk/compare/rust/v0.1.0...rust/v0.2.0) (2026-10-04)


### ⚠ BREAKING CHANGES

* request fields are optional in every language (a Rust request is built with ..Default::default(), a Go scalar is a pointer set with inorbit.Ptr); answer fields with presence (messages, optional, oneof) are optional where they used to be typed as always present.

### Features

* the contract states what the sync patched in; requests leave out unset fields ([#85](https://github.com/inorbithr/sdk/issues/85)) ([9eff311](https://github.com/inorbithr/sdk/commit/9eff311ab88bee9873a777b90197c16b471809e2))

## 0.1.0 (2026-10-03)


### Features

* **cli:** iterators over paged lists in TypeScript, Python, Rust, Java and C# ([#66](https://github.com/inorbithr/sdk/issues/66)) ([3a42fd6](https://github.com/inorbithr/sdk/commit/3a42fd63fbfc855a916e8694f0ff5c0fe4c7bab5))
* **cli:** sdk generate from profiles, sdk check with iohr.lock, profile account ([#36](https://github.com/inorbithr/sdk/issues/36)) ([748f30a](https://github.com/inorbithr/sdk/commit/748f30ab65499dd8b9d31a3e909a75bb6dfbd1ae))
* **cli:** the generator with the Rust target; the crate's public surface is generated ([#35](https://github.com/inorbithr/sdk/issues/35)) ([d4e77cf](https://github.com/inorbithr/sdk/commit/d4e77cffd5d7a20a77a484fdd56a59fbcac1dd4d))
* **repo:** six languages registered; four new conformance cases ([#42](https://github.com/inorbithr/sdk/issues/42)) ([eff831b](https://github.com/inorbithr/sdk/commit/eff831b063b31a47aa2434e7379b2af3f8e5fba2))
* **rust:** the inorbithr runtime (ADR 0011) ([#33](https://github.com/inorbithr/sdk/issues/33)) ([045054e](https://github.com/inorbithr/sdk/commit/045054e5cc751d7caa00219b5aa32508bcd5dd87))
* **spec:** the contract synced with the platform; unprocessable in every runtime ([#65](https://github.com/inorbithr/sdk/issues/65)) ([69aad11](https://github.com/inorbithr/sdk/commit/69aad110b1e77c4c49241846581959537dfcf15f))


### Bug fixes

* **conformance:** the drivers start the replay server on Windows ([#62](https://github.com/inorbithr/sdk/issues/62)) ([3dd4fdc](https://github.com/inorbithr/sdk/commit/3dd4fdcd386ad03b16dcfd09cdda1e33f069d96b))


### Documentation

* runtime and surface ([#39](https://github.com/inorbithr/sdk/issues/39)) ([d9828f2](https://github.com/inorbithr/sdk/commit/d9828f2d710fdb85074be2a0b9a2751ebdfaf552))
