# Dart package

`inorbit`, Dart 3.9 or newer, for Flutter apps and Dart servers. Read the root
`AGENTS.md` and `docs/design.md` first; this file adds only what is specific to Dart.

## What it holds today

The runtime is started, not finished (platform RFC 0074.8, slice 1):

- `lib/src/generated/contract.g.dart`: every operation's successful answer and the
  schemas it reaches, taken from `spec/openapi.json` by `tool/gen_contract.dart`
  (`mise run dart:gen`). Never edited by hand; CI regenerates it and fails on a diff.
- `ContractVerifier`: checks an answer against that contract. It is cheap and shallow on
  purpose (required fields, kinds, listed values, the first 50 items of a list), never
  throws in the default mode, and its `ContractMismatch` carries no value: an operation
  id, a path template, a field pointer and two kinds.

The client (configuration, tokens, retries, errors, the generated surface from
`iohr sdk generate --lang dart`) and the conformance driver are the next slices.

## Rules

- No runtime dependencies. Dev dependencies: `test` and `lints`.
- A mismatch never names a value, an identifier or a concrete path, and the package
  sends nothing anywhere (root rule: no telemetry). What an app does with mismatches is
  the app's choice.
- `ContractMode.strict` throws; tests and the conformance suite use it.

## Commands

- `mise run dart:check`: `dart format` check, `dart analyze --fatal-infos`, `dart test`
- `mise run dart:fmt`, `mise run dart:gen`
- `mise run dart:live`: the live API's answers checked against the contract (public
  operations; `INORBIT_API_KEY` adds the account's). Prints kinds and field paths only.
