# Changelog

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
