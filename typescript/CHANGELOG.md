# Changelog

## [0.2.2](https://github.com/inorbithr/sdk/compare/typescript/v0.2.1...typescript/v0.2.2) (2026-10-05)


### Features

* **conformance:** replay matchers, TLS, mTLS and proxy listeners, fake iohr (M6) ([#113](https://github.com/inorbithr/sdk/issues/113)) ([fe098b6](https://github.com/inorbithr/sdk/commit/fe098b64bbfec4980ec7d1f3f2efe925873498b1))
* **ts:** calls have a total timeout of 120 s by default (totalTimeout), every attempt and wait included ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** Client.load with one precedence (code, environment, the iohr config file, defaults), describe() and ConfigError problems ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** proxy, CA bundle, mTLS, pinning and connect timeout through a node:https transport, or a caller's fetch or dispatcher ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** the credential chain (env, token and key secret files, the iohr login) with CachedToken, TokenFile, CliToken, ChainedCredential and DefaultCredential ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** the named middleware pipeline with its built-ins, logging, OpenTelemetry, rate limits and the retry budget ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))


### Bug fixes

* **conformance:** the replay server hands back action options and rewrite ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))

## [0.2.1](https://github.com/inorbithr/sdk/compare/typescript/v0.2.0...typescript/v0.2.1) (2026-10-04)


### Features

* streams in all six languages, over server-sent events or the /v1/ws socket (M5) ([#96](https://github.com/inorbithr/sdk/issues/96)) ([7ff73fe](https://github.com/inorbithr/sdk/commit/7ff73fe986f4706299d188ef9a4d5273dc0ca7ee))

## [0.2.0](https://github.com/inorbithr/sdk/compare/typescript/v0.1.0...typescript/v0.2.0) (2026-10-04)


### ⚠ BREAKING CHANGES

* request fields are optional in every language (a Rust request is built with ..Default::default(), a Go scalar is a pointer set with inorbit.Ptr); answer fields with presence (messages, optional, oneof) are optional where they used to be typed as always present.

### Features

* the contract states what the sync patched in; requests leave out unset fields ([#85](https://github.com/inorbithr/sdk/issues/85)) ([9eff311](https://github.com/inorbithr/sdk/commit/9eff311ab88bee9873a777b90197c16b471809e2))

## 0.1.0 (2026-10-03)


### Features

* **cli:** iterators over paged lists in TypeScript, Python, Rust, Java and C# ([#66](https://github.com/inorbithr/sdk/issues/66)) ([3a42fd6](https://github.com/inorbithr/sdk/commit/3a42fd63fbfc855a916e8694f0ff5c0fe4c7bab5))
* **repo:** six languages registered; four new conformance cases ([#42](https://github.com/inorbithr/sdk/issues/42)) ([eff831b](https://github.com/inorbithr/sdk/commit/eff831b063b31a47aa2434e7379b2af3f8e5fba2))
* **spec:** the contract synced with the platform; unprocessable in every runtime ([#65](https://github.com/inorbithr/sdk/issues/65)) ([69aad11](https://github.com/inorbithr/sdk/commit/69aad110b1e77c4c49241846581959537dfcf15f))
* **ts:** the TypeScript runtime and target ([#46](https://github.com/inorbithr/sdk/issues/46)) ([3a2b2b0](https://github.com/inorbithr/sdk/commit/3a2b2b03f3325bac7a048cd642d6ac5c93f495b3))


### Bug fixes

* **conformance:** the drivers start the replay server on Windows ([#62](https://github.com/inorbithr/sdk/issues/62)) ([3dd4fdc](https://github.com/inorbithr/sdk/commit/3dd4fdcd386ad03b16dcfd09cdda1e33f069d96b))
