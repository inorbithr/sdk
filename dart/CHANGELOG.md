# Changelog

## [0.2.0-dev.1](https://github.com/inorbithr/sdk/compare/dart/v0.1.0-dev.1...dart/v0.2.0-dev.1) (2026-10-12)


### ⚠ BREAKING CHANGES

* the RFCs API is the Decisions API. `iohr rfc` is removed with no alias: use `iohr decisions`. The review default reads IOHR_DECISIONS_REVIEWER instead of IOHR_RFC_REVIEWER, and tokens need decisions:read / decisions:write instead of rfc:read / rfc:write.

### Features

* **dart:** the Dart runtime starts with the contract and an answer checker ([#173](https://github.com/inorbithr/sdk/issues/173)) ([689facd](https://github.com/inorbithr/sdk/commit/689facda10680a81b1e272a5cf9568184c3a7ce0))
* Decisions API, the contract synced to 136 operations, upgrade_required ([#189](https://github.com/inorbithr/sdk/issues/189)) ([91f998c](https://github.com/inorbithr/sdk/commit/91f998c9b53bb25587469e32155e893bcefe765c))
