# Changelog

## [0.2.3](https://github.com/inorbithr/sdk/compare/go/v0.2.2...go/v0.2.3) (2026-10-06)


### Documentation

* bring every README to what is released and on main ([#155](https://github.com/inorbithr/sdk/issues/155)) ([637daeb](https://github.com/inorbithr/sdk/commit/637daeb6a1b40d014773b227e218502b17a40806))

## [0.2.2](https://github.com/inorbithr/sdk/compare/go/v0.2.1...go/v0.2.2) (2026-10-05)


### Features

* **conformance:** replay matchers, TLS, mTLS and proxy listeners, fake iohr (M6) ([#113](https://github.com/inorbithr/sdk/issues/113)) ([fe098b6](https://github.com/inorbithr/sdk/commit/fe098b64bbfec4980ec7d1f3f2efe925873498b1))
* **conformance:** replay vectors prints the vectors as JSON for drivers without a YAML reader ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** calls have a total timeout of 120 s by default (WithTotalTimeout), every attempt and wait included ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** inorbit.Load with one precedence (code, environment, the iohr config file, defaults), Describe and ConfigError problems ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** OpenTelemetry spans and metrics in the optional module github.com/inorbithr/sdk/go/otel ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** proxy, CA bundle, mTLS, pinning and connect timeout through the SDK's own http.Transport, or a caller's client or RoundTripper ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** the credential chain (env, token and key secret files, the iohr login) with CachedToken, TokenFile, CliToken, ChainedCredential and DefaultCredential ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** the named middleware pipeline over http.RoundTripper with its built-ins, log/slog logging, rate limits and the retry budget ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))

## [0.2.1](https://github.com/inorbithr/sdk/compare/go/v0.2.0...go/v0.2.1) (2026-10-04)


### Features

* streams in all six languages, over server-sent events or the /v1/ws socket (M5) ([#96](https://github.com/inorbithr/sdk/issues/96)) ([7ff73fe](https://github.com/inorbithr/sdk/commit/7ff73fe986f4706299d188ef9a4d5273dc0ca7ee))

## [0.2.0](https://github.com/inorbithr/sdk/compare/go/v0.1.0...go/v0.2.0) (2026-10-04)


### ⚠ BREAKING CHANGES

* request fields are optional in every language (a Rust request is built with ..Default::default(), a Go scalar is a pointer set with inorbit.Ptr); answer fields with presence (messages, optional, oneof) are optional where they used to be typed as always present.

### Features

* the contract states what the sync patched in; requests leave out unset fields ([#85](https://github.com/inorbithr/sdk/issues/85)) ([9eff311](https://github.com/inorbithr/sdk/commit/9eff311ab88bee9873a777b90197c16b471809e2))

## 0.1.0 (2026-10-03)


### Features

* **go:** iterators over paged lists, slog logging, Example functions, virtual-clock tests ([#60](https://github.com/inorbithr/sdk/issues/60)) ([069a9cb](https://github.com/inorbithr/sdk/commit/069a9cb0bc516be82d2f290457c1455b752f669b))
* **go:** the Go runtime and target ([#56](https://github.com/inorbithr/sdk/issues/56)) ([8bbebcb](https://github.com/inorbithr/sdk/commit/8bbebcb4870fa86fbd45e55b0fee6bd1ddf871e4))
* **repo:** six languages registered; four new conformance cases ([#42](https://github.com/inorbithr/sdk/issues/42)) ([eff831b](https://github.com/inorbithr/sdk/commit/eff831b063b31a47aa2434e7379b2af3f8e5fba2))
* **spec:** the contract synced with the platform; unprocessable in every runtime ([#65](https://github.com/inorbithr/sdk/issues/65)) ([69aad11](https://github.com/inorbithr/sdk/commit/69aad110b1e77c4c49241846581959537dfcf15f))


### Bug fixes

* **conformance:** the drivers start the replay server on Windows ([#62](https://github.com/inorbithr/sdk/issues/62)) ([3dd4fdc](https://github.com/inorbithr/sdk/commit/3dd4fdcd386ad03b16dcfd09cdda1e33f069d96b))
