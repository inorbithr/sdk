# Changelog

## [0.1.0-alpha.14](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.13...iohr/v0.1.0-alpha.14) (2026-10-09)


### Features

* **cli:** iohr ext service, and install offers an extension's system service ([#186](https://github.com/inorbithr/sdk/issues/186)) ([e6783ea](https://github.com/inorbithr/sdk/commit/e6783ea7a8a5ada504dc104d1abea9bd7199284a))

## [0.1.0-alpha.13](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.12...iohr/v0.1.0-alpha.13) (2026-10-08)


### ⚠ BREAKING CHANGES

* **rust:** with the `otel` feature, `ClientBuilder::tracer_provider` and `meter_provider` take opentelemetry 0.33 types; an application on opentelemetry 0.32 upgrades it to 0.33 too.

### Features

* **cli:** a branded sign-in page that says what really happened ([#182](https://github.com/inorbithr/sdk/issues/182)) ([e5e5de4](https://github.com/inorbithr/sdk/commit/e5e5de4255d482bad107864ded6df051e9cd1038))
* **cli:** iohr browser install, the browser extension from its store ([#178](https://github.com/inorbithr/sdk/issues/178)) ([94e74c4](https://github.com/inorbithr/sdk/commit/94e74c4c3f378ef2569a25f4cf55babcb545eb83))
* **cli:** iohr rfc names PRDs and ADRs by kind (platform RFC 0081) ([#181](https://github.com/inorbithr/sdk/issues/181)) ([c30c8d4](https://github.com/inorbithr/sdk/commit/c30c8d42a617efc7155825c65259c6e153a8fdf0))


### Dependencies

* **rust:** opentelemetry 0.33, schemars held at 0.8 for typify ([#170](https://github.com/inorbithr/sdk/issues/170)) ([dd98b90](https://github.com/inorbithr/sdk/commit/dd98b9036907ab0647a20554fb65f6f4926c1605))

## [0.1.0-alpha.12](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.11...iohr/v0.1.0-alpha.12) (2026-10-07)


### Features

* **cli:** iohr ext search, show and install PUBLISHER/NAME from the catalogue ([#168](https://github.com/inorbithr/sdk/issues/168)) ([79a4661](https://github.com/inorbithr/sdk/commit/79a466144af7d32ac256642ea2f5822fb7907402))
* **cli:** iohr rfc, RFCs in the RFCs product ([#167](https://github.com/inorbithr/sdk/issues/167)) ([5da90ef](https://github.com/inorbithr/sdk/commit/5da90ef2322b7c37726fe24f2b1b22e79b679f20))

## [0.1.0-alpha.11](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.10...iohr/v0.1.0-alpha.11) (2026-10-06)


### Features

* **cli:** classified spans and review keys in iohr lab check ([#153](https://github.com/inorbithr/sdk/issues/153)) ([a8bacc7](https://github.com/inorbithr/sdk/commit/a8bacc7b35c60a477af4751ed692e3dd8d024949))
* **cli:** extensions declare privileges, confirmed before install ([#151](https://github.com/inorbithr/sdk/issues/151)) ([fdf0784](https://github.com/inorbithr/sdk/commit/fdf07841bb537a3af13921028c7ab541bb5aff9a))


### Documentation

* bring every README to what is released and on main ([#155](https://github.com/inorbithr/sdk/issues/155)) ([637daeb](https://github.com/inorbithr/sdk/commit/637daeb6a1b40d014773b227e218502b17a40806))

## [0.1.0-alpha.10](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.9...iohr/v0.1.0-alpha.10) (2026-10-05)


### Features

* **cli:** iohr sdk add installs the SDK with the project's package manager ([#144](https://github.com/inorbithr/sdk/issues/144)) ([9473dfe](https://github.com/inorbithr/sdk/commit/9473dfeea44bf1bc9a8e0ad2b91f3d23a8d59699))
* **cli:** iohr sdk config resolves through the Rust SDK's loader, and the Rust target emits the idempotency-key mark and the path template ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **cli:** the C# target emits the path template and the idempotency-key mark ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **conformance:** replay vectors prints the vectors as JSON for drivers without a YAML reader ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **csharp:** calls have a total timeout of 120 s by default (TotalTimeout), every attempt and wait included ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** Client.Load with one precedence (code, environment, the iohr config file, defaults), Describe() and ConfigException problems ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** proxy, CA bundle, mTLS, pinning and connect timeout on SocketsHttpHandler, or a caller's HttpClient or handler ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** the credential chain (env, token and key secret files, the iohr login) with CachedToken, TokenFile, CliToken, ChainedCredential and DefaultCredential ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** the named middleware pipeline with its built-ins, Middleware.FromHandler, ILogger logging, ActivitySource and Meter, rate limits and the retry budget ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **csharp:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([4b3299c](https://github.com/inorbithr/sdk/commit/4b3299cec13358be44e850d96c0384bd34f63d58))
* **go:** calls have a total timeout of 120 s by default (WithTotalTimeout), every attempt and wait included ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** inorbit.Load with one precedence (code, environment, the iohr config file, defaults), Describe and ConfigError problems ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** OpenTelemetry spans and metrics in the optional module github.com/inorbithr/sdk/go/otel ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** proxy, CA bundle, mTLS, pinning and connect timeout through the SDK's own http.Transport, or a caller's client or RoundTripper ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** the credential chain (env, token and key secret files, the iohr login) with CachedToken, TokenFile, CliToken, ChainedCredential and DefaultCredential ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** the named middleware pipeline over http.RoundTripper with its built-ins, log/slog logging, rate limits and the retry budget ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **go:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([1564943](https://github.com/inorbithr/sdk/commit/1564943e2e4a7daa4f7ad7475e8742590f8f8682))
* **java:** calls have a total timeout of 120 s by default, and the attempt timeout covers the body ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** Client.load with one precedence (code, environment, the iohr config file, defaults), describe() and ConfigException problems ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** proxy, CA bundle, mTLS, pinning and connect timeout through the SDK's own ProxySelector and SSLContext ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** the credential chain (env, token and key secret files, the iohr login) with TokenFile, CliToken, CachedToken, ChainedCredential and DefaultCredential ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** the named middleware pipeline with its built-ins, System.Logger logging, optional OpenTelemetry, rate limits and the retry budget ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **py:** calls have a 120 s total deadline by default (total_timeout) ([3c8075d](https://github.com/inorbithr/sdk/commit/3c8075d922cd8341dd80815636d7936d56815cdd))
* **py:** load, the credential chain, the middleware pipeline, transport settings, logging and OpenTelemetry (M6) ([3c8075d](https://github.com/inorbithr/sdk/commit/3c8075d922cd8341dd80815636d7936d56815cdd))
* **py:** writes that take Idempotency-Key are retried, with one key per call ([3c8075d](https://github.com/inorbithr/sdk/commit/3c8075d922cd8341dd80815636d7936d56815cdd))
* **rust:** calls have a total timeout of 120 s by default (total_timeout), and a Retry-After longer than retry_after_max ends the call ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** Client::load with one precedence (code, environment, the iohr config file, defaults), describe() and ConfigError problems ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** proxy, CA bundle, mTLS, pinning and connect timeout through the SDK's own rules, or a caller's reqwest::Client ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** the credential chain (env, token and key secret files, the iohr login) with TokenFile, CliToken, CachedToken, ChainedCredential and DefaultCredential ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** the named middleware pipeline with its built-ins, tracing logging, OpenTelemetry behind the otel feature, rate limits and the retry budget ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **rust:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))
* **ts:** calls have a total timeout of 120 s by default (totalTimeout), every attempt and wait included ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** Client.load with one precedence (code, environment, the iohr config file, defaults), describe() and ConfigError problems ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** proxy, CA bundle, mTLS, pinning and connect timeout through a node:https transport, or a caller's fetch or dispatcher ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** the credential chain (env, token and key secret files, the iohr login) with CachedToken, TokenFile, CliToken, ChainedCredential and DefaultCredential ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** the named middleware pipeline with its built-ins, logging, OpenTelemetry, rate limits and the retry budget ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **ts:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))


### Bug fixes

* **conformance:** the replay server hands back action options and rewrite ([1fe4d5a](https://github.com/inorbithr/sdk/commit/1fe4d5ad42675a7ed1efa63334bc8bfa768a282f))
* **py:** the attempt timeout covers the body; the socket sends an iohr- request id and uses the proxy; the token exchange sends the SDK's user agent; from_env normalises profile names ([3c8075d](https://github.com/inorbithr/sdk/commit/3c8075d922cd8341dd80815636d7936d56815cdd))
* **rust:** the socket task ends when the client is dropped ([10aeae0](https://github.com/inorbithr/sdk/commit/10aeae0e4ece4e6881e3604a74ed45391aaf5288))


### Dependencies

* **rust:** tokio-tungstenite 0.30, pkcs8 0.11 and fresh lockfiles ([#135](https://github.com/inorbithr/sdk/issues/135)) ([d13b761](https://github.com/inorbithr/sdk/commit/d13b761ad7b96924c1d14bea08ecd3cd3517aace))

## [0.1.0-alpha.9](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.8...iohr/v0.1.0-alpha.9) (2026-10-05)


### Features

* **cli:** iohr sdk config prints the SDKs' effective configuration ([#110](https://github.com/inorbithr/sdk/issues/110)) ([e4a9d9c](https://github.com/inorbithr/sdk/commit/e4a9d9cef7652fc179f384074b3e42cb801d23a5))
* **cli:** the generator marks operations that take Idempotency-Key ([#111](https://github.com/inorbithr/sdk/issues/111)) ([1390526](https://github.com/inorbithr/sdk/commit/1390526e579e4c213cebca9cff75ab1ed0a3a0fc))


### Bug fixes

* **cli:** sdk config's path rules and vector tests hold on Windows ([#120](https://github.com/inorbithr/sdk/issues/120)) ([c4fdefd](https://github.com/inorbithr/sdk/commit/c4fdefd2912ce8ac709805f55f7e7de82589d9e0))

## [0.1.0-alpha.8](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.7...iohr/v0.1.0-alpha.8) (2026-10-04)


### Features

* **cli:** iohr sdk examples, one compile-checked snippet per operation and language ([#107](https://github.com/inorbithr/sdk/issues/107)) ([f0b871e](https://github.com/inorbithr/sdk/commit/f0b871e14112179b4caed8285e387ad015972af6))
* **cli:** share config.toml with the SDKs, add iohr auth token ([#109](https://github.com/inorbithr/sdk/issues/109)) ([16cb8ef](https://github.com/inorbithr/sdk/commit/16cb8ef26303385fedda8ce3fcbfa4235b2b2606))


### Bug fixes

* **cli:** IOHR_TOKEN wins over the default profile, as the README and ADR 0009 say ([16cb8ef](https://github.com/inorbithr/sdk/commit/16cb8ef26303385fedda8ce3fcbfa4235b2b2606))

## [0.1.0-alpha.7](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.6...iohr/v0.1.0-alpha.7) (2026-10-04)


### Features

* **cli:** iohr connectors and iohr connections (RFC 0044) ([#105](https://github.com/inorbithr/sdk/issues/105)) ([a964223](https://github.com/inorbithr/sdk/commit/a96422376ddcd970c4b18900d1b0349639d497ba))
* streams in all six languages, over server-sent events or the /v1/ws socket (M5) ([#96](https://github.com/inorbithr/sdk/issues/96)) ([7ff73fe](https://github.com/inorbithr/sdk/commit/7ff73fe986f4706299d188ef9a4d5273dc0ca7ee))

## [0.1.0-alpha.6](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.5...iohr/v0.1.0-alpha.6) (2026-10-04)


### Features

* **cli:** iohr api --all walks every page of a list into one answer ([#95](https://github.com/inorbithr/sdk/issues/95)) ([ea86b94](https://github.com/inorbithr/sdk/commit/ea86b94fc45b30f2833c1ad850efb28e20759a20))

## [0.1.0-alpha.5](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.4...iohr/v0.1.0-alpha.5) (2026-10-04)


### ⚠ BREAKING CHANGES

* request fields are optional in every language (a Rust request is built with ..Default::default(), a Go scalar is a pointer set with inorbit.Ptr); answer fields with presence (messages, optional, oneof) are optional where they used to be typed as always present.

### Features

* the contract states what the sync patched in; requests leave out unset fields ([#85](https://github.com/inorbithr/sdk/issues/85)) ([9eff311](https://github.com/inorbithr/sdk/commit/9eff311ab88bee9873a777b90197c16b471809e2))

## [0.1.0-alpha.4](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.3...iohr/v0.1.0-alpha.4) (2026-10-03)


### Features

* **cli:** iterators over paged lists in TypeScript, Python, Rust, Java and C# ([#66](https://github.com/inorbithr/sdk/issues/66)) ([3a42fd6](https://github.com/inorbithr/sdk/commit/3a42fd63fbfc855a916e8694f0ff5c0fe4c7bab5))
* **csharp:** the C# runtime and target ([#58](https://github.com/inorbithr/sdk/issues/58)) ([bc1e7bb](https://github.com/inorbithr/sdk/commit/bc1e7bbcb0927247e31d754a92020a153489995b))
* **go:** iterators over paged lists, slog logging, Example functions, virtual-clock tests ([#60](https://github.com/inorbithr/sdk/issues/60)) ([069a9cb](https://github.com/inorbithr/sdk/commit/069a9cb0bc516be82d2f290457c1455b752f669b))
* **go:** the Go runtime and target ([#56](https://github.com/inorbithr/sdk/issues/56)) ([8bbebcb](https://github.com/inorbithr/sdk/commit/8bbebcb4870fa86fbd45e55b0fee6bd1ddf871e4))
* **java:** the Java runtime and the Java target ([#59](https://github.com/inorbithr/sdk/issues/59)) ([325f2f9](https://github.com/inorbithr/sdk/commit/325f2f953bfa0e985bf45ff0203ae7dd7452ca78))
* **py:** the Python runtime and target ([#57](https://github.com/inorbithr/sdk/issues/57)) ([e823d2e](https://github.com/inorbithr/sdk/commit/e823d2e9f4e91c29ae14d7ff0efa71be416f9a15))
* **spec:** the contract synced with the platform; unprocessable in every runtime ([#65](https://github.com/inorbithr/sdk/issues/65)) ([69aad11](https://github.com/inorbithr/sdk/commit/69aad110b1e77c4c49241846581959537dfcf15f))
* **ts:** the TypeScript runtime and target ([#46](https://github.com/inorbithr/sdk/issues/46)) ([3a2b2b0](https://github.com/inorbithr/sdk/commit/3a2b2b03f3325bac7a048cd642d6ac5c93f495b3))


### Bug fixes

* **cli:** a new platform error code no longer stops sdk generate ([#61](https://github.com/inorbithr/sdk/issues/61)) ([b6689ef](https://github.com/inorbithr/sdk/commit/b6689ef6611663164e03d113b7684b9ead0ec7e2))

## [0.1.0-alpha.3](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.2...iohr/v0.1.0-alpha.3) (2026-10-03)


### Features

* **cli:** a language-neutral generator core; six languages known ([#41](https://github.com/inorbithr/sdk/issues/41)) ([e6952a0](https://github.com/inorbithr/sdk/commit/e6952a000a7c7193906c16da2f0440918da317e1))
* **cli:** iohr ext (RFC 0028) and iohr domains (RFC 0030) ([#40](https://github.com/inorbithr/sdk/issues/40)) ([dbae5c8](https://github.com/inorbithr/sdk/commit/dbae5c8cb211c7decb652df910f90d476e35951d))
* **cli:** iohr lab check, the lab document checks on your own repository ([#53](https://github.com/inorbithr/sdk/issues/53)) ([9cb02be](https://github.com/inorbithr/sdk/commit/9cb02be784c5f45b9096228defd192cf00755e12))
* **cli:** sdk generate from profiles, sdk check with iohr.lock, profile account ([#36](https://github.com/inorbithr/sdk/issues/36)) ([748f30a](https://github.com/inorbithr/sdk/commit/748f30ab65499dd8b9d31a3e909a75bb6dfbd1ae))
* **cli:** the generator with the Rust target; the crate's public surface is generated ([#35](https://github.com/inorbithr/sdk/issues/35)) ([d4e77cf](https://github.com/inorbithr/sdk/commit/d4e77cffd5d7a20a77a484fdd56a59fbcac1dd4d))


### Bug fixes

* **cli:** the ext tests build on Windows ([#51](https://github.com/inorbithr/sdk/issues/51)) ([1ee3f8b](https://github.com/inorbithr/sdk/commit/1ee3f8b004c863def0540132b6e81d4d49cef056))
* **cli:** the Homebrew formula's description passes brew audit ([#29](https://github.com/inorbithr/sdk/issues/29)) ([7535a35](https://github.com/inorbithr/sdk/commit/7535a35602182773be9b08e8353d4c55ba6248f9))
* **cli:** the last Unix-only import in the ext tests moves with its helper ([#52](https://github.com/inorbithr/sdk/issues/52)) ([87c3cfd](https://github.com/inorbithr/sdk/commit/87c3cfdb3838eb115b5da6d714414e47c456c75d))

## [0.1.0-alpha.2](https://github.com/inorbithr/sdk/compare/iohr/v0.1.0-alpha.1...iohr/v0.1.0-alpha.2) (2026-10-02)


### Bug fixes

* **cli:** the release scripts live in cli/release ([#27](https://github.com/inorbithr/sdk/issues/27)) ([eef71a8](https://github.com/inorbithr/sdk/commit/eef71a87b8f8bf395e50279339c9f9190507b244))

## 0.1.0-alpha.1 (2026-10-02)


### Features

* **cli:** install.sh and install.ps1, checked against the release ([#18](https://github.com/inorbithr/sdk/issues/18)) ([7cff577](https://github.com/inorbithr/sdk/commit/7cff57755e379325b9ca04232ea6767e8a52b6bd))
* **cli:** iohr login in a browser or with a device code ([#15](https://github.com/inorbithr/sdk/issues/15)) ([5387abc](https://github.com/inorbithr/sdk/commit/5387abc8d5b838c12b92729485fc375e35152918))
* **cli:** iohr signs in with an API token, keeps profiles and calls the API ([#14](https://github.com/inorbithr/sdk/issues/14)) ([6c3189c](https://github.com/inorbithr/sdk/commit/6c3189c62df5ebfeb612be21a244f5802509872c))
