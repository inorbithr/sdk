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

1. Your plan: `iohr project init` made it (or found the one it made before) and wrote its id
   under `[plan]` in `coordination.toml`. If you were signed out it is empty: sign in, then
   `iohr coordinator plan create @@NAME@@ --external-id project:@@NAME@@` and put the `plan_...`
   id there. `iohr coordinator plan show PLAN` lists its items and notes.
2. File the example ask (it queues work for the coordinator's planner, it runs nothing here):

```sh
iohr coordinator asks create --kind chore --title "First task for @@NAME@@" \
  --body "Add a second health check to @@NAME@@" --external-id @@NAME@@-first
iohr coordinator asks list --state open
```

The same `--external-id` returns the same ask, so filing it twice is harmless.

## Build through the Planner

Every build step is a Planner item, so the plan shows what ran, who holds it and how it ended:

```sh
iohr coordinator item run PLAN build --title "Build @@NAME@@" -- cargo build   # or any command
iohr coordinator item add PLAN docs --title "Write the docs"                   # an item to do
iohr coordinator item claim PLAN docs                                          # take it
iohr coordinator item note PLAN docs "outline done"
iohr coordinator item done PLAN docs --note "published"                        # or: item fail
```

`item run` adds the item when it is missing, claims it, runs the command, then notes and
marks it done, or notes the failure and puts it back to `proposed` (the Planner has no
failed status). A step already `done` is skipped, so a build can resume.

## Run it

```sh
iohr ext lint --preview     # the manifest, checked, in plain words
iohr ext dev                # run it with live logs, a real token channel, $STATE and $CACHE
iohr ext dev --test         # the self-test
```
