# Runner

This machine as a coordinator runner: what `iohr runner` does, how it signs in to the model, and what it keeps. The command table is in [the README](../README.md#commands).

```sh
iohr runner api-key set                 # once: your Anthropic API key, no echo
iohr runner register                    # this machine, by its host name
iohr runner run                         # until Ctrl-C
iohr runner status
```

To run as Io instead of a person (platform RFC 0065.2, so jobs Io files run on runners Io
owns), add `--sign-in io` (or `IOHR_SIGN_IN=io`) to `iohr runner`. The runner then signs
in as the OAuth2 client `iohr-io` with the client credentials grant, asks for
`coordinator:runner coordinator:read coordinator:write approvals:read approvals:write`,
keeps the 15-minute token in memory and mints a new one shortly before it ends. The secret
is never an argument or an environment value: it comes from the file `IOHR_CLIENT_SECRET_FILE`
names (it must be yours and mode 0600, or the runner stops) or, on a Mac, from the
keychain item the Mac bridge uses (`iohr-mcp-bridge-io`). A profile (`--profile`, else the
default) still holds the runner's keys and settings, and `--account` names the account.
Nothing else changes: without the flag the runner signs in as your profile does.
`--sign-in io` also works on `iohr coordinator ...` (every other command refuses it), so an
agent's jobs and asks are Io's, not the person's who runs it. To say which role made
the call, add `--via-role NAME` (`IOHR_VIA_ROLE`; the old `--via-personality` and `IOHR_VIA_PERSONALITY` still work), and optionally
`--via-session ID` (`IOHR_VIA_SESSION`) and `--via-job ID` (`IOHR_VIA_JOB`): they set the
`x-iohr-via` header (`personality=improver; session=...; job=...`), which is attribution only and
grants nothing. Each value is 1 to 64 of `A-Z a-z 0-9 . _ : -`, else the call is refused. (`--via` is
taken: `iohr ask --via` says where an answer comes from.) These flags need `--sign-in io`.
Example: `iohr --sign-in io --via-role improver coordinator jobs create --kind chore --brief -`.
A runner of Io's that holds a person's provider sign-in (the person signed in to `claude`
on that machine) names that person at registration:
`iohr --sign-in io runner register --holder <subject>`. The person must be Io's seat
holder and already hold the same sign-in on a runner of their own; the coordinator then
gives that runner only their verified work.

The runner is this machine's half of a coordinator turn. It signs in to InOrbit as your
`iohr` profile does, which needs the scopes `coordinator:runner coordinator:read`
(`iohr login --scope coordinator:runner,coordinator:read`), and calls the coordinator's
runner calls: register (it makes an Ed25519 key, kept in the credential store your
profile uses, the macOS Keychain on a Mac, and signs the service's challenge with it),
heartbeat (every `heartbeat_interval_seconds` the service sends, 20 s; `--interval`
overrides), accept a lease, get its turn pack, keep the lease alive while the turn
runs (the turn stops when the service says so), and report the signed result. A report
the service does not take is kept in memory and retried; it is lost if the runner exits
first. A turn is one
`claude -p` run of the `claude` first on your PATH, as you installed it:
`--output-format json`, the turn's `--model` and `--max-turns`, its `--allowedTools`, the
pack as the system prompt, and `--resume <session>` only when this runner ran the
previous turn of the thread and the session is on its disk (otherwise
`--session-id <new>`). The prompt goes in on stdin. To keep the turn's context small it
also passes `--setting-sources project,local` and `--strict-mcp-config`, so your own
user-level hooks, plugins, skills and MCP servers are not loaded into a turn (your
sign-in is). On `claude` 2.1.307 one short turn went from 33,220 to 15,054 context tokens.

It runs on an Anthropic API key, the path for organisations and teams. Your own Claude
plan is for personal use only: it is off unless the runner table says
`claude_auth = "any"` and you accepted the notice `iohr runner personal-use` shows, and
then the runner takes one turn at a time, only work you assigned yourself
(`docs/config.md` section 4.3). `claude` sends its requests to Anthropic directly;
InOrbit does not proxy them.

- Before every turn it asks the binary (`claude auth status`) and refuses unless it says
  `api_key` (or, where allowed, `claude.ai`) and `firstParty`. It never reads Claude
  Code's own credential files and never signs in for you.
- The stored key reaches only the `claude` process the runner starts, as
  `ANTHROPIC_API_KEY` with `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB=1` beside it; it is never
  an argument, never printed, never logged and never sent to InOrbit. A key or token
  given as an argument to `iohr` is refused before anything runs.
- The child gets an allowlist of your environment, nothing else: `PATH`, `HOME`, `USER`,
  `LOGNAME`, `SHELL`, `TERM`, `TMPDIR`, `TZ`, `LANG`, `LC_*`, `CLAUDE_CONFIG_DIR` and the
  XDG directories (and the Windows equivalents), so `claude` finds its own sign-in.
  Anything whose name holds `TOKEN`, `KEY`, `SECRET` or `PASSWORD` is dropped even if
  listed, and so are `ANTHROPIC_*`, `AWS_*`, `GOOGLE_*`, `OPENAI_*`, `AZURE_*`, `GH_TOKEN`,
  `GITHUB_TOKEN` and `IOHR_TOKEN*`. There is no option to pass one through.
- When the runner stops (Ctrl-C or revocation), the running turn is aborted and
  `claude` and everything it started are killed.
- It never passes `--bare`, `--settings` or an endpoint, and never edits the binary or
  its requests. Both the arguments and the environment are table-tested, and a `--bare`
  or an `ANTHROPIC_BASE_URL` in their output fails the build.
- A turn is refused when its user is not the person the runner is registered to, and on
  your own Claude plan when someone else assigned it.
- `gh` and git are signed out in the child: `GH_CONFIG_DIR` points at a directory that
  does not exist, git's credential helpers are cleared (`GIT_CONFIG_COUNT`) and its
  prompts are off (`GIT_TERMINAL_PROMPT=0`). The check a job runs gets the same.

## GitHub access

A job whose repository is on GitHub is checked out, pushed to `job/<job_id>` and opened
as a pull request by the runner itself, never by `claude`. For each lease it asks the
platform for a GitHub App installation token (platform RFC 0120): that one repository,
`contents` and `pull_requests` write, about an hour; it asks again when fewer than
10 minutes are left before a push. The token is kept in memory only. git gets it per
command through a credential helper given with `-c` (every helper from your own git
configuration is dropped for that command, and hooks are off), only in the runner's own
mirror under its working directory after the mirror's configuration is checked; `gh`
gets it as `GH_TOKEN` for that one call. It is never written to a file or a git
configuration and never logged.

Your own `gh` sign-in is used instead only when the platform says its App is not
installed for the account, or the platform is older than the call; the log says which.
Any other refusal (the App lacks a permission, the lease was not confirmed) leaves the
lease unaccepted, so another runner can take it.

## Session mode

`iohr session` is the way into a project's Roles (platform RFC 0213). Opening it starts
one host process for the project: `iohr runner run --session DIR`, which is the same runner
(heartbeats, the 90 s server-side lease, turns on your `claude -p`, no self-upgrade while it
hosts) plus the project's Roles at work on one local queue. The host stops when the last
session has left, when a shell that was killed has left, on `iohr stop`, and on SIGTERM
(exit 0). It keeps no more than one host per project: a lock the kernel frees when the
process dies, so a crash leaves nothing to clean up.

The queue is a DuckDB file, `queue.duckdb`, in `session/<project hash>/` below the data
directory (`IOHR_DATA_DIR`, else the platform's own), beside the host's socket and log
(`host.log`, the host's own output). The host is the only process that opens it; shells talk
to the host over its socket. The folder must be yours and mode 0700 (the host refuses one that
is not, or that is a link) and the socket is mode 0600; a socket path over 103 bytes is refused
with a message naming the fix, and a host holds at most 64 connections (more are answered
`error busy`). The queue holds `roles`, `tasks` and an append-only `task_events`. A task is
`queued`, `claimed`, `running`, then `done` or `failed`; a Role takes the next task in one
statement, with a lease of 90 s that its heartbeat renews. A statement that meets another
Role's edit of the same row is tried again, not failed.

Leases are timed on a monotonic clock of the host process, never the wall clock, so a laptop
that sleeps or a clock that is set back cannot make a live Role's task look abandoned. While a
host runs, a task is taken from its Role only when that Role's thread has ended and its lease
has run out; a check every few seconds does that and also fails a task that was tried 3 times.
A host that starts holds the lock alone, so every claim left in the file is an orphan: each
goes back to `queued` with the attempt counted, and after 3 attempts becomes `failed` with the
reason. A task is meant to be worked once at a time; a host killed in the middle of one runs
it again on the next start (at least once), so a handler must be safe to repeat.

`iohr stop` makes the host drain: a Role takes no more tasks, an answer it is still writing is
cut short (its child process group is killed) and the task goes back to `queued` with the try not
counted, and until the last Role has stopped the host still answers `status` with `stopping` and
holds its lock. `--wait` polls that lock, so "stopped" is said only when the host has really
ended, and "stopping" when the wait ran out first.

Asks (slice S2 of RFC 0213). When a host runs for the folder an ask is made in (or one above it),
each `?`/`??` ask is put in the queue once per Role (`external_id` `ask:<id>:<role>`, so a repeat
adds nothing), for every Role the host runs, or only the ones `--role a,b` names. Each Role
answers as a child of the host, `iohr --json ask --headless --role <Role> --cwd <folder>` (the
way `--role` answers today; read-only, `plan` when the ask was `plan`), at the same time as
Io's own answer. When Io's card is drawn, the Roles' answers follow, one card each as it lands
(the stored answer is the task's `result_ref`: JSON `{text, model, duration_ms}`, the text cut
at 48K characters). A Role that fails shows its reason in its card. Ctrl-C leaves the wait; the
Roles keep working and `iohr stop` ends them. A task still queued or cut short when the host
stops is answered by the next host. With no host, with `--json`, `--menu`, `--dry-run`,
`--allow-edits` or `--mode auto`, with a `--role` the host does not run, or with an older host,
the ask goes the way it always did. The socket's `roles` and `results <ask>` requests carry
this. Nothing yet re-attaches to an ask's answers after the wait ended, and finished tasks are
not pruned. No launchd or systemd unit starts a session host at login.

Each Role keeps two things in the project between its turns (RFC 0213 S2). When a Role's turn
ends the host writes its handoff note, `.inorbit/handoff/<role>.md`: what it was asked and what
it answered (cut to 800 and 2000 characters, replaced by each turn, written through a temporary
file and a rename). Its memory area is `.inorbit/memory/<role>/`, the same memory files as
`iohr memory` (the project's own store skips the sub-folder, so a Role's lessons are not the
project's). Every `iohr ask --role <Role>` turn, routed or not, is told the note and the newest
eight memories of the area (about 2000 characters, pinned first) after the Role's text. A Role
name that is not lower-case letters, digits, `-` and `_` (at most 64) keeps and reads nothing.
Both are plain files in the project folder; nothing is sent anywhere. Nothing writes the memory
area yet: a person adds a memory file there by hand, in the format of `.inorbit/memory/`.

## Staying current

`run` looks for a newer `iohr` release about hourly, and when there is one it takes no new
work, waits for the running turns and approved runs to finish, installs it with the
verified install of `iohr upgrade`, and restarts itself (`exec` of the same path, or exit
75 for a launchd or systemd supervisor with `upgrade_restart = "exit"`). A failure keeps
the old version running and backs off from 5 minutes. `--no-auto-upgrade` or
`auto_upgrade = false` turns it off; a copy installed by apt is reported and skipped.
On macOS, `upgrade_sign_identity = "auto"` (or `IOHR_SIGN_IDENTITY`) signs each new binary
with a code-signing identity from your keychain, so a keychain "Always Allow" grant holds
across upgrades; it is off by default. Keys, back-off and limits are in `docs/config.md`
section 4.3.

## Capacity

How many turns the runner takes at once, and which classes, come from the `[runner]`
table of `config.toml` (or `[profiles.<name>.runner]`), and `--capacity` and `--kinds`
override it:

```toml
[runner]
capacity = 4                      # 1 to 32
kinds = ["coordinate", "build"]
```

`run` sends them on every heartbeat, so a change reaches the coordinator within one beat;
`kill -HUP <pid>` makes a running `run` read the file again. Lowering the capacity never
stops a running turn. Below 100 GB free on `/mnt/build` the runner takes no new build
turn until there is room. `iohr runner config show` prints what is in effect and where
each value came from (`docs/config.md`, section 4.3).

## Gate report

`iohr runner gate-report --job J --lease L --token N` is the runner's half of the Checked
gate (platform RFC 0120). In the job's checkout (`--repo`, default the current directory)
it fetches `origin/main`, refuses a checkout with uncommitted changes or one that does not
contain `origin/main`, runs `--command` (default `mise run ci:changed`) through the shell
with the scrubbed environment above and a `--timeout` (default 1800 s, at most 21600 s), and
sends the coordinator the command, its exit code, the head, main's commit and the SHA-256
of the output, signed with the key this runner registered. The output itself stays on this
machine; when the check fails, its last 4 KB (whole lines, token-shaped words and URL
credentials redacted) is also printed here (`output_tail` with `--json`) and written to the
runner's log, so a job held at Checked says why. The tail is never signed or sent. A non-zero exit is sent as it is: the job holds at Checked, which is the honest
answer. A coordinator that does not have the call yet (an older release) is reported as
such and is not an error. `iohr runner run` does the same by itself: when a job turn ends
with a reply, it runs this check (the same command, timeout and report) in the checkout the
turn's pack names (`checkout_path`, `checkouts/<job_id>`, relative to the runner's working
directory for turns; a path that is absolute or climbs out of it is refused), keeps the lease beating while it runs, and sends the
report before the turn's own report. It sends nothing, and logs why, when the turn did not
reply, the pack names no checkout (a job with no repository, or an older coordinator), the
check cannot run (dirty or unmerged checkout, time cap) or the lease is gone. Call the
command yourself only for a job the loop did not check.

What the sign-in service sees of the provider is the SHA-256 of the account's salt, a
newline and the identity line, and the method's name. The identity line is built from
`claude auth status` alone, from the fields that name the seat:
`authMethod=claude.ai apiProvider=firstParty orgId=<orgId> email=<email>`, with `orgId`
and `email` trimmed and lowercased. Fields that change on a token refresh or with the host
(`loginExpiresAt`, `configDirectory`, `projectsDirectory`) and those that change without
changing the seat (`orgName`, `subscriptionType`) stay out, so the hash is stable across
token refresh. A sign-in whose status names no organisation or no email is refused at
registration. `iohr runner register --refresh` sends the registration again with the key
this machine holds, after signing in to another account or to update the runner; the
runner keeps its id.

## Approved run asks

`iohr runner run` also carries out run asks the owner approved and pinned to this runner
(platform RFC 0172 and RFC 0153). It needs two things set up once, and stays off with a
note saying which is missing:

- A run key. `iohr runner run-key` makes a P-256 key, kept in the credential store your
  profile uses (like the coordinator key `register` makes, and never printed). Not enrolled
  yet, it sends an enrolment ask to your phone (showing the key's label and fingerprint),
  waits up to `--wait` minutes (15) for your approval and enrols the key; run it again to keep
  waiting on the same ask. To enrol from a file instead, have your device sign the enrolment
  message (`iohr-approval-runner-v1`, the public key, the label, the time, a nonce and the
  device key id), save that proof as JSON `{key_id, signed_at, nonce, signature}`, and run
  `iohr runner run-key --proof FILE`: the runner adds its own signature of possession and
  enrols the key. `run-key` needs no
  coordinator scope.
- The device keys it trusts, in `runner/approvals/<account>/device-keys` in the config directory:
  one `key_id public_key` per line (the key base64url of the 65-byte P-256 point), `#`
  for a comment. The service does not list a person's public keys, so they are pinned
  here. The file, like the record below, must belong to you and not be writable by group
  or world (`chmod go-w`), or the runner stays off.
- The cluster, `kube_context` and/or `kubeconfig` under `[runner]` (or a profile's
  runner table). Every `kubectl` call gets `--context` and `--kubeconfig` from them, so an
  approved action reaches only that cluster; with neither set, every action is refused.

Every 20 seconds (up to 5 minutes after errors) the runner lists the account's approved
`run` asks and takes those pinned to its own runner id. Before anything runs it checks,
in the order the platform checks them, that the ask is approved and pinned, that the
payload hashes to what was approved, that a person decided it, that the action fits the
`runner/host-action/v0` schema, that it names this runner and its key, that it has not
expired, and that a pinned device key signed the decision. An ask whose decision this
machine has already carried out (the record is `runner/approvals/<account>/seen`, owner-only, one line per
decision, a malformed line is an error) is refused as a replay. A refused ask is reported `failed` with the reason and runs nothing.
Otherwise the runner records the decision on disk (under a file lock), reports `started`,
runs the action and reports `done` or `failed` with at most 500 characters of what
happened, with addresses, URLs and tokens redacted; each report is signed with the run key.

The catalogue has one action, `k8s.delete-failed-pods`: `kubectl delete pods
--field-selector=status.phase=Failed` in the namespace the ask names, or in every
namespace. It runs `kubectl` from an absolute `PATH` entry with an argument vector (never
a shell), the scrubbed environment above (no `KUBECONFIG`: the configured cluster is used), and a 2-minute limit. A
namespace must be a DNS label, and `kube-system`, `kube-public` and `kube-node-lease` are
refused. Any other action is reported `failed` as unknown.
