# @@ID@@

An InOrbit extension: a program `iohr` runs as its own process, with a token channel
that hands out only the API scopes its manifest declares. This one writes a heartbeat
to `$STATE/heartbeat.json` every minute and asks for a token with `identity:read`.

Scaffolded by `iohr ext init @@ID@@ --lang @@LANG@@`.

## What it may do

`iohr ext lint --preview` prints this from `extension.toml`, the way an installer shows
it before anything runs. As scaffolded:

- May ask iohr for API tokens with identity:read, and nothing else.
- May read and write its own state and cache, nowhere else.
- Reaches no network host and starts no other program.
- Asks for no secrets.
- Sends up its health (state, heartbeat time, version), nothing else.

Change `extension.toml` and run the preview again: what you declare there is what a
person agrees to when they install it.

## Develop

```sh
iohr ext lint --preview     # the manifest, checked, in plain words
iohr ext dev                # run it with live logs, a real token channel, $STATE and $CACHE
iohr ext dev --test         # run tests/ with the same channel and folders
```

`iohr ext dev` keeps its state in `~/.inorbit/ext-dev/@@NAME@@/` and marks heartbeats
`"dev": true`. It never touches the extensions you installed. A token request for a
scope the manifest does not declare is refused and shown in red: that is how you learn
what the extension really needs.

Secrets are references: declare `{ name, ref, env }` in `extension.toml`, and while you
develop give the value as `IOHR_EXT_SECRET_<NAME>` or keep it in the Keychain (service
`hr.inorbit.iohr.ext`, account `<ref>`). Never put a value in the manifest; the lint
refuses anything that looks like one.

@@LANG_NOTES@@

## Release

Tag `v<version>` (the version in `extension.toml`) and push the tag.
`.github/workflows/release.yml` builds the program, derives the artifact's config with
`iohr ext lint --json`, pushes the OCI artifact to `ghcr.io/<owner>/iohr-ext/@@NAME@@`
and attaches a keyless cosign signature and SLSA provenance, both from that workflow at
the tag, which is what `iohr ext install` verifies.
