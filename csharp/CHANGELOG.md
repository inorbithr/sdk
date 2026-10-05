# Changelog

## [0.2.2](https://github.com/inorbithr/sdk/compare/csharp/v0.2.1...csharp/v0.2.2) (2026-10-05)


### Features

* **cli:** the C# target emits the path template and the idempotency-key mark ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **conformance:** replay matchers, TLS, mTLS and proxy listeners, fake iohr (M6) ([#113](https://github.com/inorbithr/sdk/issues/113)) ([fe098b6](https://github.com/inorbithr/sdk/commit/fe098b64bbfec4980ec7d1f3f2efe925873498b1))
* **csharp:** calls have a total timeout of 120 s by default (TotalTimeout), every attempt and wait included ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** Client.Load with one precedence (code, environment, the iohr config file, defaults), Describe() and ConfigException problems ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** proxy, CA bundle, mTLS, pinning and connect timeout on SocketsHttpHandler, or a caller's HttpClient or handler ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** the credential chain (env, token and key secret files, the iohr login) with CachedToken, TokenFile, CliToken, ChainedCredential and DefaultCredential ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** the named middleware pipeline with its built-ins, Middleware.FromHandler, ILogger logging, ActivitySource and Meter, rate limits and the retry budget ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))


### Dependencies

* **csharp:** Logging.Abstractions 10, xUnit v3 and the test SDK 18 ([#141](https://github.com/inorbithr/sdk/issues/141)) ([89efb8d](https://github.com/inorbithr/sdk/commit/89efb8db590d95ca5c1c4857d27d388b002435d2))

## [0.2.1](https://github.com/inorbithr/sdk/compare/csharp/v0.2.0...csharp/v0.2.1) (2026-10-04)


### Features

* streams in all six languages, over server-sent events or the /v1/ws socket (M5) ([#96](https://github.com/inorbithr/sdk/issues/96)) ([7ff73fe](https://github.com/inorbithr/sdk/commit/7ff73fe986f4706299d188ef9a4d5273dc0ca7ee))

## [0.2.0](https://github.com/inorbithr/sdk/compare/csharp/v0.1.0...csharp/v0.2.0) (2026-10-04)


### ⚠ BREAKING CHANGES

* request fields are optional in every language (a Rust request is built with ..Default::default(), a Go scalar is a pointer set with inorbit.Ptr); answer fields with presence (messages, optional, oneof) are optional where they used to be typed as always present.

### Features

* **cli:** iterators over paged lists in TypeScript, Python, Rust, Java and C# ([#66](https://github.com/inorbithr/sdk/issues/66)) ([3a42fd6](https://github.com/inorbithr/sdk/commit/3a42fd63fbfc855a916e8694f0ff5c0fe4c7bab5))
* **csharp:** the C# runtime and target ([#58](https://github.com/inorbithr/sdk/issues/58)) ([bc1e7bb](https://github.com/inorbithr/sdk/commit/bc1e7bbcb0927247e31d754a92020a153489995b))
* **repo:** six languages registered; four new conformance cases ([#42](https://github.com/inorbithr/sdk/issues/42)) ([eff831b](https://github.com/inorbithr/sdk/commit/eff831b063b31a47aa2434e7379b2af3f8e5fba2))
* **spec:** the contract synced with the platform; unprocessable in every runtime ([#65](https://github.com/inorbithr/sdk/issues/65)) ([69aad11](https://github.com/inorbithr/sdk/commit/69aad110b1e77c4c49241846581959537dfcf15f))
* the contract states what the sync patched in; requests leave out unset fields ([#85](https://github.com/inorbithr/sdk/issues/85)) ([9eff311](https://github.com/inorbithr/sdk/commit/9eff311ab88bee9873a777b90197c16b471809e2))
