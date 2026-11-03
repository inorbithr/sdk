# @@NAME@@

Your own project, built on InOrbit: a private macOS service that heartbeats and serves a tiny
local panel. It was made from the `demo` template by InOrbit Desktop (or `iohr project init`),
and it is isolated from everything else on this Mac: its own folder, its own state under
`~/.inorbit/projects/@@NAME@@/`, its own token scope, and nothing of yours.

## What is here

- `extension.toml`: the manifest (PRD 0084). What it runs, what it may do, how it is healthy.
- `src/@@MODULE@@/`: the service. `__main__.py` heartbeats and serves the panel; `channel.py`
  asks iohr's token socket for a token with the one scope the manifest declares.
- `panel/index.html`: the panel InOrbit Desktop shows as a pane. It reads `/api/heartbeat`.
- `tests/test_selftest.py`: the self-test. One heartbeat, nothing written outside its state.
- `inorbit-project.toml`: the project record the app reads (name, template, kind, when).

## Run it

```sh
iohr ext lint --preview     # the manifest, checked, in plain words
iohr ext dev                # run it with live logs, a real token channel, $STATE and $CACHE
iohr ext dev --test         # the self-test, with the same channel and folders
```

Without the SDK's dev loop installed, InOrbit Desktop's smoke test runs it once:

```sh
STATE=~/.inorbit/projects/@@NAME@@/state CACHE=~/.inorbit/projects/@@NAME@@/cache \
IOHR_EXT_ONCE=1 PYTHONPATH=src python3 -m @@MODULE@@
```

## What it may do

As scaffolded, and as `iohr ext lint --preview` says:

- May ask iohr for API tokens with `identity:read`, and nothing else.
- May read and write its own state and cache, nowhere else.
- Reaches no network host; the panel listens on 127.0.0.1 only.
- Asks for no secrets.
- Sends up its health (state, heartbeat time, version), nothing else.

Change `extension.toml` and run the preview again. What you declare there is what a person
agrees to when the project is installed, on this Mac or anywhere else.

## Next

Code it in the app's Code page (it is an allowed repository, in its own sandbox). Packaging
and installing it on this Mac (`iohr ext version`, `pack`, `sign`, `install --target this-mac`,
PRD 0084 slice 2) and publishing it privately to the Marketplace from the app are planned, not
built yet.
