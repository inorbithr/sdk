# Runner

`iohr runner` makes this machine one of your account's coordinator runners: it takes turns
from the coordinator and runs them on your own Claude Code subscription. This page covers
setting one up, how a turn runs, GitHub access, keeping the runner current, capacity, the
gate report and approved run asks. Back to the [iohr overview](../README.md).

```sh
claude                                  # once: /login with your subscription
iohr runner register --name laptop      # this machine; the default name is its host name
iohr runner run                         # until Ctrl-C
iohr runner status                      # your runners as the service sees them
```

The runner signs in to InOrbit as your `iohr` profile does, which needs the scopes
`coordinator:runner` and `coordinator:read`:

```sh
iohr login --scope coordinator:runner,coordinator:read
```

`iohr session` runs a runner too, as a project's session host:
[Sessions and the panel](session-and-panel.md).

## The commands

| Command | What it does |
|---|---|
| `iohr runner register [--name N] [--capacity 1-32] [--always-on] [--refresh]` | Register this machine. Run again, it says the runner exists; `--refresh` sends the registration again with the key this machine holds, after signing in to another account or to update it, and the runner keeps its id |
| `iohr runner run [--name N \| --runner ID] [--interval S] [--capacity N] [--kinds K] [--no-auto-upgrade] [--max-beats N]` | The loop, until Ctrl-C. `--runner` is also `IOHR_RUNNER_ID`; `--max-beats` stops after that many answered heartbeats, for scripts |
| `iohr runner status` | Your runners as the service sees them, and what this machine can run |
| `iohr runner config show [--capacity N] [--kinds K] [--no-auto-upgrade]` | The values `run` would use now, each with where it came from; needs no sign-in |
| `iohr runner gate-report --job J --lease L --token N` | Run a job's check and send a signed report ([below](#gate-report)) |
| `iohr runner run-key [--label L] [--wait M] [--proof FILE] [--show]` | This machine's run key for approved run asks ([below](#approved-run-asks)) |

The runner's settings live in the `[runner]` table of `config.toml` (or
`[profiles.<name>.runner]`): [docs/config.md](../../docs/config.md), section 4.3.

## How a turn runs

The runner is this machine's half of a coordinator turn. It calls the coordinator's runner
calls in this order:

1. **Register.** It makes an Ed25519 key, kept in the credential store your profile uses
   (the macOS Keychain on a Mac), and signs the service's challenge with it.
2. **Heartbeat**, every `heartbeat_interval_seconds` the service sends (20 seconds today);
   `--interval` (1 to 300) overrides it.
3. **Accept a lease** and get the turn's pack.
4. **Run the turn**, keeping the lease alive while it runs. The turn stops when the service
   says so.
5. **Report** the signed result. A report the service does not take is kept in memory and
   retried; it is lost if the runner exits first.

A turn is one `claude -p` run of the `claude` first on your `PATH`, as you installed it:

- `--output-format json`, the turn's `--model`, `--max-turns` and `--allowedTools`, and the
  pack as the system prompt. The prompt goes in on stdin.
- `--resume <session>` only when this runner ran the previous turn of the thread and the
  session is on its disk; otherwise `--session-id <new>`.
- `--setting-sources project,local` and `--strict-mcp-config`, so your own user-level
  hooks, plugins, skills and MCP servers are not loaded into a turn. Your sign-in is.

### On your subscription only

- Before every turn it asks the binary (`claude auth status`) and refuses unless the answer
  says `claude.ai` and `firstParty`. It never reads Claude Code's own credential files and
  never signs in for you.
- It never passes `--bare`, `--settings` or an endpoint, and never edits the binary or its
  requests. The arguments and the environment are table-tested, and a `--bare` or an
  `ANTHROPIC_BASE_URL` in them fails the build.
- A turn is refused when its user is not the person the runner is registered to.
- When the runner stops (Ctrl-C or revocation), the running turn is aborted and `claude`
  and everything it started are killed.

The child gets an allowlist of your environment and nothing else: `PATH`, `HOME`, `USER`,
`LOGNAME`, `SHELL`, `TERM`, `TMPDIR`, `TMP`, `TEMP`, `TZ`, `LANG`, `LANGUAGE`, `LC_*`,
`CLAUDE_CONFIG_DIR`, the XDG directories, `CARGO_TARGET_DIR` (which the runner sets per
repository) and the Windows equivalents, so `claude` finds its own sign-in.

- A name that holds `TOKEN`, `KEY`, `SECRET`, `PASSWORD`, `PASSWD` or `CREDENTIAL` is
  dropped even when it is on the list.
- So are the provider families: `ANTHROPIC_*`, `AWS_*`, `OPENAI_*`, `CODEX_*`,
  `AZURE_OPENAI_*`, `GOOGLE_CLOUD_*`, `GOOGLE_GENAI_*`, `GCLOUD_*`, `VERTEX_*`, the
  `CLAUDE_CODE_USE_*` routing switches, and `IOHR_TOKEN*`.
- There is no option to pass one through.

`gh` and git are signed out in the child: `GH_CONFIG_DIR` points at a directory that does
not exist, git's credential helpers are cleared and its prompts are off
(`GIT_TERMINAL_PROMPT=0`). The check a job runs gets the same.

### What the service learns of your sign-in

Registration sends a SHA-256 of the account's salt, a newline and one identity line, and the
method's name. The line is built from `claude auth status` alone, from the fields that name
the seat:

```text
authMethod=claude.ai apiProvider=firstParty orgId=<orgId> email=<email>
```

`orgId` and `email` are trimmed and lower-cased. Fields that change on a token refresh or
with the host (`loginExpiresAt`, `configDirectory`, `projectsDirectory`) and those that
change without changing the seat (`orgName`, `subscriptionType`) stay out, so the hash is
the same across token refreshes. A status that names no organisation or no email is refused
at registration.

## Signing in as Io

To run as Io, the account's agent identity, instead of a person, so the jobs Io files run on
runners Io owns, add `--sign-in io` (or `IOHR_SIGN_IN=io`):

```sh
iohr --sign-in io runner run
```

- The runner signs in as the OAuth2 client `iohr-io` with the client credentials grant and
  asks for `coordinator:runner coordinator:read coordinator:write approvals:read
  approvals:write`. It keeps the 15-minute token in memory and gets a new one shortly
  before it ends.
- The secret is never an argument or an environment value. It comes from the file
  `IOHR_CLIENT_SECRET_FILE` names (yours, mode 0600, or the runner stops) or, on a Mac,
  from the keychain item the InOrbit Mac app's bridge uses.
- A profile (`--profile`, else the default) still holds the runner's keys and settings, and
  `--account` names the account. Without the flag the runner signs in as your profile does.
- A runner of Io's that uses a person's provider sign-in (the person signed in to `claude`
  on that machine) names that person at registration with `--holder SUBJECT`. That person
  must already hold the same sign-in on a runner of their own; the runner then takes only
  their verified work.

`--sign-in io` also works on `iohr coordinator`, and every other command refuses it. The
attribution flags that go with it (`--via-role`, `--via-session`, `--via-job`) are in
[Commands](commands.md).

## GitHub access

A job whose repository is on GitHub is checked out, pushed to `job/<job_id>` and opened as
a pull request by the runner itself, never by `claude`.

- For each lease the runner asks the platform for a GitHub App installation token: that one
  repository, `contents` and `pull_requests` write, about an hour. It asks again when fewer
  than 10 minutes are left before a push.
- The token is kept in memory only. git gets it per command through a credential helper
  given with `-c` (every helper from your own git configuration is dropped for that command,
  and hooks are off), only in the runner's own mirror after the mirror's configuration is
  checked. `gh` gets it as `GH_TOKEN` for that one call. It is never written to a file or a
  git configuration and never logged.
- Your own `gh` sign-in is used instead only when the platform says its App is not
  installed for the account, or the platform is older than the call; the log says which.
  Any other refusal (the App lacks a permission, the lease was not confirmed) leaves the
  lease unaccepted, so another runner can take it.

Job checkouts are in `runner/coordinate/checkouts/<job_id>` in the config directory, as
worktrees of one mirror per repository (`runner/mirrors/<owner>/<repo>.git`). A job's check
and its model's `cargo` build into one shared `CARGO_TARGET_DIR` per repository,
`runner/target/<owner>/<repo>`, so the dependencies are on disk once. Cargo locks that
directory, so two builds of one repository take turns.

## Staying current

A running runner keeps itself on the newest `iohr` release, and this is on by default.
What it checks before it installs one, how often it looks and how to turn it off are in
[SECURITY.md](../../SECURITY.md), section "Automatic upgrades"; this page adds only how a
runner behaves around an upgrade.

| What | Value |
|---|---|
| Before installing | Takes no new work and waits up to 4 hours for the running turns and approved runs to finish |
| After a failure | The old version keeps running; the wait backs off from 5 minutes, doubling, up to 6 hours, and never longer than the interval |
| Restart | `exec` of the same path, or exit 75 for a launchd or systemd supervisor with `upgrade_restart = "exit"` |

On macOS, `upgrade_sign_identity = "auto"` (or `IOHR_SIGN_IDENTITY`) signs each new binary
with a code-signing identity from your keychain, so a keychain "Always Allow" grant holds
across upgrades. It is off by default.

The keys and their limits: [docs/config.md](../../docs/config.md), section 4.3.

## Capacity

How many turns the runner takes at once, and which classes, come from the `[runner]` table
of `config.toml` (or `[profiles.<name>.runner]`); `--capacity` and `--kinds` override it:

```toml
[runner]
capacity = 4                      # 1 to 32
kinds = ["coordinate", "build"]
```

- `run` sends them on every heartbeat, so a change reaches the coordinator within one beat.
  `kill -HUP <pid>` makes a running `run` read the file again.
- Lowering the capacity never stops a running turn.
- Build turns pause while the build volume, when the machine has one, has under 100 GB
  (100 × 10^9 bytes) free. `iohr runner config show` prints the volume and the floor.

## Gate report

`iohr runner gate-report --job J --lease L --token N` is the runner's half of the job's
Checked gate. In the job's checkout (`--repo`, default the current directory) it:

1. fetches `origin/main` and refuses a checkout with uncommitted changes, or one that does
   not contain `origin/main`;
2. runs `--command` (default `mise run ci:changed`) through the shell with the scrubbed
   environment above and a `--timeout` (1 to 21,600 seconds, default 1,800);
3. sends the coordinator the command, its exit code, the head, main's commit and the
   SHA-256 of the output, signed with the key this runner registered.

The output itself stays on this machine. When the check fails, its last 4,096 bytes (whole
lines, with token-shaped words and URL credentials redacted) are printed here
(`output_tail` with `--json`) and written to the runner's log, so a job held at Checked says
why. The tail is never signed or sent.

A non-zero exit is sent as it is, and the job holds at Checked. A coordinator that does not
have the call yet (an older release) is reported as such and is not an error.

`iohr runner run` does the same by itself when a job turn ends with a reply: the same
command, timeout and report, in the checkout the turn's pack names (`checkout_path`,
relative to the runner's working directory; a path that is absolute or climbs out of it is
refused). It keeps the lease beating while the check runs and sends the report before the
turn's own. It sends nothing, and logs why, when the turn did not reply, the pack names no
checkout, the check cannot run (a dirty or unmerged checkout, the time cap) or the lease is
gone. Call the command yourself only for a job the loop did not check.

## Approved run asks

`iohr runner run` also carries out run asks you approved and pinned to this runner. It needs
three things set up once, and stays off with a note that says which is missing.

**A run key.** `iohr runner run-key` makes a P-256 key, kept in the credential store your
profile uses beside the runner key and never printed.

- Not enrolled yet, it sends an enrolment ask to your phone that shows the key's label and
  fingerprint, waits up to `--wait` minutes (default 15, 0 to 60; 0 files the ask and
  returns) and enrols the key. Run it again to keep waiting on the same ask.
- To enrol from a file instead, have your device sign the enrolment message
  (`iohr-approval-runner-v1`, the public key, the label, the time, a nonce and the device key
  id), save that proof as JSON `{key_id, signed_at, nonce, signature}`, and run
  `iohr runner run-key --proof FILE`. The runner adds its own signature of possession.
- `--label` is up to 100 characters (default `iohr runner on` and the host name). `--show`
  prints the key and its enrolled runner id and makes and asks nothing. `run-key` needs no
  coordinator scope.

**The device keys it trusts**, in `runner/approvals/<account>/device-keys` in the config
directory: one `key_id public_key` per line (the key is the base64url of the 65-byte P-256
point), `#` for a comment. The service does not list a person's public keys, so they are
pinned here. The file, like the record below, must be yours and not writable by group or
world (`chmod go-w`), or the runner stays off.

**The cluster**: `kube_context` and/or `kubeconfig` under `[runner]` (or a profile's runner
table). Every `kubectl` call gets `--context` and `--kubeconfig` from them, so an approved
action reaches only that cluster. With neither set, every Kubernetes action is refused and nothing runs.

### How an approved ask runs

Every 20 seconds (backing off up to 300 seconds after errors) the runner lists the account's
approved `run` asks and takes those pinned to its own runner id. Before anything runs it
checks, in the order the platform checks them, that:

- the ask is approved and pinned, and its payload hashes to what was approved;
- a person decided it;
- the action fits the `runner/host-action/v0` schema and names this runner and its key;
- it has not expired;
- a pinned device key signed the decision;
- this machine has not carried out that decision before. The record is
  `runner/approvals/<account>/seen`, one line per decision; a malformed line is an error.

A refused ask is reported `failed` with the reason and runs nothing. Otherwise the runner
records the decision on disk (under a file lock), reports `started`, runs the action and
reports `done` or `failed` with at most 500 characters of what happened, with addresses,
URLs and tokens redacted. Each report is signed with the run key.

The actions are a fixed catalogue built into the binary; an ask cannot name a program of its
own. Each action runs its program from an absolute `PATH` entry with an argument vector
(never a shell), the scrubbed environment above (no `KUBECONFIG`: the configured cluster is
used) and a time limit of its own, from 120 to 3,600 seconds. A `kubectl` action that takes
a namespace needs a DNS label, and `kube-system`, `kube-public` and `kube-node-lease` are
refused. An action that is not in the catalogue is reported `failed` as unknown.
