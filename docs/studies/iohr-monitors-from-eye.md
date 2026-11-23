# iohr monitors, moved from Eye (RFC 0234, slice S3)

Date: 2026-11-23. Plan item `s3-monitors` of plan_01M83R9J7125P8X01VWPZ13K4Q.

## Asked
Move Eye's repo monitor files into the `iohr` CLI as `iohr monitors` (check, run, list,
show, history, enable, disable, delete, export, install), stacked on the schema check (S1).

## Found
- Eye's `entry()` rules mirror the dataplane agent's `checks_file.rs`; kept to type monitors,
  while the real `agent checks lint` is spawned on install.
- Eye's own schema loader, validator and `schema_id` duplicate the SDK `schema.rs`; replaced.
- Eye's `term` styling duplicates `ui.rs`; replaced. Date helpers (`human`, `secs`, `iso`)
  could use chrono or `time`; left as is.
- No second checks.toml parser exists in the SDK.
- Eye's `shapes.rs` tests Eye's own routes and stays in Eye.

## Decided
- State in `.inorbit/state/monitors`, self-ignored by a `.gitignore` containing `*`, so the
  project template's own `.gitignore` is untouched.
- Install lints the candidate with the real agent before swapping it in. The lint program is
  `IOHR_MONITORS_LINT`, else this binary, else `iohr` on PATH. Shared install helpers live in
  `runner/host_action/install_checks.rs`.
- No endpoints, tools or scopes are added. Pre-1.0 semver: a feat is a patch.

## Built
`src/monitors/*`, `src/commands/monitors.rs`, the `monitors` project-init step and a disabled
`example.toml`, 8 integration tests in `tests/monitors.rs`, docs in `cli/docs/commands.md` and
`cli/docs/projects.md`.

## Open
A cli check runs `iohr` unless `IOHR_MONITORS_BIN` is set; the lint needs the agent installed.
