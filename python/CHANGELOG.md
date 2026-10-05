# Changelog

## [0.2.2](https://github.com/inorbithr/sdk/compare/python/v0.2.1...python/v0.2.2) (2026-10-05)


### Features

* **conformance:** replay matchers, TLS, mTLS and proxy listeners, fake iohr (M6) ([#113](https://github.com/inorbithr/sdk/issues/113)) ([fe098b6](https://github.com/inorbithr/sdk/commit/fe098b64bbfec4980ec7d1f3f2efe925873498b1))
* **py:** calls have a 120 s total deadline by default (total_timeout) ([3c8075d](https://github.com/inorbithr/sdk/commit/3c8075d922cd8341dd80815636d7936d56815cdd))
* **py:** load, the credential chain, the middleware pipeline, transport settings, logging and OpenTelemetry (M6) ([3c8075d](https://github.com/inorbithr/sdk/commit/3c8075d922cd8341dd80815636d7936d56815cdd))
* **py:** writes that take Idempotency-Key are retried, with one key per call ([3c8075d](https://github.com/inorbithr/sdk/commit/3c8075d922cd8341dd80815636d7936d56815cdd))


### Bug fixes

* **py:** the attempt timeout covers the body; the socket sends an iohr- request id and uses the proxy; the token exchange sends the SDK's user agent; from_env normalises profile names ([3c8075d](https://github.com/inorbithr/sdk/commit/3c8075d922cd8341dd80815636d7936d56815cdd))


### Dependencies

* **py:** allow websockets 17 and lock 17.2 ([#138](https://github.com/inorbithr/sdk/issues/138)) ([614c9fd](https://github.com/inorbithr/sdk/commit/614c9fdf3e0e96f01997a17664927e7bc141a671))

## [0.2.1](https://github.com/inorbithr/sdk/compare/python/v0.2.0...python/v0.2.1) (2026-10-04)


### Features

* streams in all six languages, over server-sent events or the /v1/ws socket (M5) ([#96](https://github.com/inorbithr/sdk/issues/96)) ([7ff73fe](https://github.com/inorbithr/sdk/commit/7ff73fe986f4706299d188ef9a4d5273dc0ca7ee))

## [0.2.0](https://github.com/inorbithr/sdk/compare/python/v0.1.0...python/v0.2.0) (2026-10-04)


### ⚠ BREAKING CHANGES

* request fields are optional in every language (a Rust request is built with ..Default::default(), a Go scalar is a pointer set with inorbit.Ptr); answer fields with presence (messages, optional, oneof) are optional where they used to be typed as always present.

### Features

* the contract states what the sync patched in; requests leave out unset fields ([#85](https://github.com/inorbithr/sdk/issues/85)) ([9eff311](https://github.com/inorbithr/sdk/commit/9eff311ab88bee9873a777b90197c16b471809e2))

## 0.1.0 (2026-10-03)


### Features

* **cli:** iterators over paged lists in TypeScript, Python, Rust, Java and C# ([#66](https://github.com/inorbithr/sdk/issues/66)) ([3a42fd6](https://github.com/inorbithr/sdk/commit/3a42fd63fbfc855a916e8694f0ff5c0fe4c7bab5))
* **py:** the Python runtime and target ([#57](https://github.com/inorbithr/sdk/issues/57)) ([e823d2e](https://github.com/inorbithr/sdk/commit/e823d2e9f4e91c29ae14d7ff0efa71be416f9a15))
* **repo:** six languages registered; four new conformance cases ([#42](https://github.com/inorbithr/sdk/issues/42)) ([eff831b](https://github.com/inorbithr/sdk/commit/eff831b063b31a47aa2434e7379b2af3f8e5fba2))
* **spec:** the contract synced with the platform; unprocessable in every runtime ([#65](https://github.com/inorbithr/sdk/issues/65)) ([69aad11](https://github.com/inorbithr/sdk/commit/69aad110b1e77c4c49241846581959537dfcf15f))


### Bug fixes

* **conformance:** the drivers start the replay server on Windows ([#62](https://github.com/inorbithr/sdk/issues/62)) ([3dd4fdc](https://github.com/inorbithr/sdk/commit/3dd4fdcd386ad03b16dcfd09cdda1e33f069d96b))
