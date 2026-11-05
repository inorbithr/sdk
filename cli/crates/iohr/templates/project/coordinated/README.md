# @@NAME@@

Your own project, built on InOrbit and wired for coordination: the `demo` service (a private
macOS program that heartbeats and serves a tiny local panel) plus `coordination.toml`, which
names the Planner plan your account's coordinator works from and shows one ask you can file.
It was made from the `coordinated` template by `iohr project init` (or `iohr project create`).

## What is here

- Everything the `demo` template has: `extension.toml`, `src/@@MODULE@@/`, `panel/`, `tests/`
  and `inorbit.toml`.
- `coordination.toml`: the plan this project belongs to and an `ask` example. Nothing in it is
  a secret; the plan id is empty until you set it.

## Wire it up

1. Find your plan: `iohr coordinator --help` lists what the coordinator offers; the plan id
   starts with `plan_`. Put it in `coordination.toml` under `[plan]`.
2. File the example ask (it queues work for the coordinator's planner, it runs nothing here):

```sh
iohr coordinator asks create --kind chore --title "First task for @@NAME@@" \
  --body "Add a second health check to @@NAME@@" --external-id @@NAME@@-first
iohr coordinator asks list --state open
```

The same `--external-id` returns the same ask, so filing it twice is harmless.

## Run it

```sh
iohr ext lint --preview     # the manifest, checked, in plain words
iohr ext dev                # run it with live logs, a real token channel, $STATE and $CACHE
iohr ext dev --test         # the self-test
```
