# Changelog

## [0.3.0](https://github.com/inorbithr/sdk/compare/java/v0.2.2...java/v0.3.0) (2026-10-09)


### ⚠ BREAKING CHANGES

* **spec:** sync the public contract, 111 operations (2026-10-07) ([#166](https://github.com/inorbithr/sdk/issues/166))
* **spec:** in Rust, the generated answer structs gain fields (Account.probe, Agent.capabilities, Use.connection_id and more) and the Surface trait gains rfcs() and trails(); code that builds those structs with a struct literal, or implements Surface itself, must add the new fields or methods. In Go, CheckDetail is no longer comparable with == (it gained the Invariants slice). Calls, names and wire shapes are unchanged in every language.

### Features

* **spec:** sync the public contract, 107 operations (2026-10-06) ([#159](https://github.com/inorbithr/sdk/issues/159)) ([1b4ebf3](https://github.com/inorbithr/sdk/commit/1b4ebf3b5b9ddfef077f4437eed16f1489f60797))
* **spec:** sync the public contract, 111 operations (2026-10-07) ([#166](https://github.com/inorbithr/sdk/issues/166)) ([f3ce0cd](https://github.com/inorbithr/sdk/commit/f3ce0cd164154ed13f96303d453765ed45c6ce90))


### Documentation

* bring every README to what is released and on main ([#155](https://github.com/inorbithr/sdk/issues/155)) ([637daeb](https://github.com/inorbithr/sdk/commit/637daeb6a1b40d014773b227e218502b17a40806))

## [0.2.2](https://github.com/inorbithr/sdk/compare/java/v0.2.1...java/v0.2.2) (2026-10-05)


### Features

* **conformance:** replay matchers, TLS, mTLS and proxy listeners, fake iohr (M6) ([#113](https://github.com/inorbithr/sdk/issues/113)) ([fe098b6](https://github.com/inorbithr/sdk/commit/fe098b64bbfec4980ec7d1f3f2efe925873498b1))
* **java:** calls have a total timeout of 120 s by default, and the attempt timeout covers the body ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** Client.load with one precedence (code, environment, the iohr config file, defaults), describe() and ConfigException problems ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** proxy, CA bundle, mTLS, pinning and connect timeout through the SDK's own ProxySelector and SSLContext ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** the credential chain (env, token and key secret files, the iohr login) with TokenFile, CliToken, CachedToken, ChainedCredential and DefaultCredential ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** the named middleware pipeline with its built-ins, System.Logger logging, optional OpenTelemetry, rate limits and the retry budget ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))
* **java:** writes whose operation takes an Idempotency-Key are now retried, with one key per call sent on every attempt ([27e9ff8](https://github.com/inorbithr/sdk/commit/27e9ff8c914fbc33f17e794d4440b0adec037fec))

## [0.2.1](https://github.com/inorbithr/sdk/compare/java/v0.2.0...java/v0.2.1) (2026-10-04)


### Features

* streams in all six languages, over server-sent events or the /v1/ws socket (M5) ([#96](https://github.com/inorbithr/sdk/issues/96)) ([7ff73fe](https://github.com/inorbithr/sdk/commit/7ff73fe986f4706299d188ef9a4d5273dc0ca7ee))

## [0.2.0](https://github.com/inorbithr/sdk/compare/java/v0.1.0...java/v0.2.0) (2026-10-04)


### ⚠ BREAKING CHANGES

* request fields are optional in every language (a Rust request is built with ..Default::default(), a Go scalar is a pointer set with inorbit.Ptr); answer fields with presence (messages, optional, oneof) are optional where they used to be typed as always present.

### Features

* **cli:** iterators over paged lists in TypeScript, Python, Rust, Java and C# ([#66](https://github.com/inorbithr/sdk/issues/66)) ([3a42fd6](https://github.com/inorbithr/sdk/commit/3a42fd63fbfc855a916e8694f0ff5c0fe4c7bab5))
* **java:** the Java runtime and the Java target ([#59](https://github.com/inorbithr/sdk/issues/59)) ([325f2f9](https://github.com/inorbithr/sdk/commit/325f2f953bfa0e985bf45ff0203ae7dd7452ca78))
* **repo:** six languages registered; four new conformance cases ([#42](https://github.com/inorbithr/sdk/issues/42)) ([eff831b](https://github.com/inorbithr/sdk/commit/eff831b063b31a47aa2434e7379b2af3f8e5fba2))
* **spec:** the contract synced with the platform; unprocessable in every runtime ([#65](https://github.com/inorbithr/sdk/issues/65)) ([69aad11](https://github.com/inorbithr/sdk/commit/69aad110b1e77c4c49241846581959537dfcf15f))
* the contract states what the sync patched in; requests leave out unset fields ([#85](https://github.com/inorbithr/sdk/issues/85)) ([9eff311](https://github.com/inorbithr/sdk/commit/9eff311ab88bee9873a777b90197c16b471809e2))
