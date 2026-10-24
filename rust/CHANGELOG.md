# Changelog

## [0.4.0](https://github.com/inorbithr/sdk/compare/rust/v0.3.0...rust/v0.4.0) (2026-10-24)


### ⚠ BREAKING CHANGES

* **rust:** a model or query-parameter struct can no longer be built with a struct literal or `..Default::default()` outside the crate; use `Model::default().with_field(value)` (or `Model::new(..)`). A match on a generated enum needs a `_` arm. Surface cannot be implemented outside the generated surface.

### Features

* **rust:** models and query parameters are non_exhaustive, built with setters; Surface is sealed ([#180](https://github.com/inorbithr/sdk/issues/180)) ([918625a](https://github.com/inorbithr/sdk/commit/918625a9b2a4e02f2f58499cc5546cd7c759e9b0))

## [0.3.0](https://github.com/inorbithr/sdk/compare/rust/v0.2.2...rust/v0.3.0) (2026-10-24)


### ⚠ BREAKING CHANGES

* the RFCs API is the Decisions API. `iohr rfc` is removed with no alias: use `iohr decisions`. The review default reads IOHR_DECISIONS_REVIEWER instead of IOHR_RFC_REVIEWER, and tokens need decisions:read / decisions:write instead of rfc:read / rfc:write.
* **rust:** with the `otel` feature, `ClientBuilder::tracer_provider` and `meter_provider` take opentelemetry 0.33 types; an application on opentelemetry 0.32 upgrades it to 0.33 too.
* **spec:** sync the public contract, 111 operations (2026-10-07) ([#166](https://github.com/inorbithr/sdk/issues/166))
* **spec:** in Rust, the generated answer structs gain fields (Account.probe, Agent.capabilities, Use.connection_id and more) and the Surface trait gains rfcs() and trails(); code that builds those structs with a struct literal, or implements Surface itself, must add the new fields or methods. In Go, CheckDetail is no longer comparable with == (it gained the Invariants slice). Calls, names and wire shapes are unchanged in every language.

### Features

* **cli:** --env and IOHR_ENV target production or local ([#208](https://github.com/inorbithr/sdk/issues/208)) ([3d44e90](https://github.com/inorbithr/sdk/commit/3d44e90345c0dba35d187a33f82ef685a85bce96))
* **cli:** iohr ext init, dev and lint, the local extension loop (PRD 0084 slice 1) ([#191](https://github.com/inorbithr/sdk/issues/191)) ([9083c23](https://github.com/inorbithr/sdk/commit/9083c23036e34e9bdc897b8839f331246366998a))
* **cli:** runner capacity and kinds from config.toml, declared on every heartbeat ([#244](https://github.com/inorbithr/sdk/issues/244)) ([2a01d11](https://github.com/inorbithr/sdk/commit/2a01d113e1be2a0d84cf5388fd7606578d86002d))
* Decisions API, the contract synced to 136 operations, upgrade_required ([#189](https://github.com/inorbithr/sdk/issues/189)) ([91f998c](https://github.com/inorbithr/sdk/commit/91f998c9b53bb25587469e32155e893bcefe765c))
* **spec:** sync the public contract, 107 operations (2026-10-06) ([#159](https://github.com/inorbithr/sdk/issues/159)) ([1b4ebf3](https://github.com/inorbithr/sdk/commit/1b4ebf3b5b9ddfef077f4437eed16f1489f60797))
* **spec:** sync the public contract, 111 operations (2026-10-07) ([#166](https://github.com/inorbithr/sdk/issues/166)) ([f3ce0cd](https://github.com/inorbithr/sdk/commit/f3ce0cd164154ed13f96303d453765ed45c6ce90))


### Bug fixes

* **cli:** the runner's run loop sends the gate report for a finished job turn ([#254](https://github.com/inorbithr/sdk/issues/254)) ([3346863](https://github.com/inorbithr/sdk/commit/3346863b9de6f6393f9cf00ea057a87f09c0ea08))
* **deps:** ctutils 0.4.5 in every Cargo lock and opentelemetry-api 1.45.1 in the Python lock so the advisory audits pass again ([#256](https://github.com/inorbithr/sdk/issues/256)) ([91446a5](https://github.com/inorbithr/sdk/commit/91446a5fea6fd4769d6d7fe4bd2c55de6b7b5316))
* extension tokens for a person, and a wait that outlasts the channel ([#205](https://github.com/inorbithr/sdk/issues/205)) ([b57dbef](https://github.com/inorbithr/sdk/commit/b57dbef00126617a197ccdbd378b2170696cac2d))


### Dependencies

* **rust:** bump the rust group across 4 directories with 14 updates ([#238](https://github.com/inorbithr/sdk/issues/238)) ([d2c7787](https://github.com/inorbithr/sdk/commit/d2c778777c522e32641c7afb4f7b6ee1c7465b51))
* **rust:** opentelemetry 0.33, schemars held at 0.8 for typify ([#170](https://github.com/inorbithr/sdk/issues/170)) ([dd98b90](https://github.com/inorbithr/sdk/commit/dd98b9036907ab0647a20554fb65f6f4926c1605))

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
