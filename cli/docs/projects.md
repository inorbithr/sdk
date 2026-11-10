# Projects

How `iohr project init` sets up a project you build yourself: the templates and how to
write your own, the project your agent builds from a description, reading your public
site, the files a project holds, the atlas and `iohr sync`, and the project's space in
InOrbit. The built-in roles and memory it writes are on
[Roles and memory](roles-and-memory.md). Back to the [iohr overview](../README.md).

## `iohr project init`

```sh
iohr project init my-shop         # the coordinated template: a Planner plan comes with it
iohr project init demo --template demo
iohr project create my-shop --template coordinated --dir ~/work/my-shop --json
```

`iohr project init NAME` (also `iohr init NAME` and `iohr project create NAME`, the same
command) sets up a project with one command and tells you what you now have. The local
half:

- a folder from a template: `--dir` if you pass it, else a menu of places at a terminal
  (`./NAME`, this folder when it is empty, beside the git repository you are in so the
  project does not nest, and `[project] root` in `config.toml` when set), else `./NAME`.
  The default template, `coordinated`, is a private macOS service that heartbeats and
  serves a tiny panel, with the manifest of `iohr ext init`, plus a `coordination.toml`
  that names the project's Planner plan and an `ask` example you can file with
  `iohr coordinator asks create`. `--template demo` is the same without the
  `coordination.toml`;
- a git repository with one commit;
- the project's own state under `~/.inorbit/projects/NAME/`, never in the repository;
- a Keychain slot reserved by name;
- a smoke test that runs the project once and reads its heartbeat back. It runs with no
  token channel, so its token ask is refused: the proof that the channel is its only
  credential.

Run it again in the folder it made and it succeeds. A file that is already there is left
alone (reported as unchanged, or as left alone when you changed it), nothing is written
twice and nothing of yours is committed for you. A folder that is not that project is
still refused. The template's `rust-version` is the toolchain you have installed, and an
agent started with `--describe` is told to keep it as the minimum.

At a terminal, `init` is guided. It asks for the name when it is missing (the folder's name
is the default), offers the templates as a menu (templates that are coming are listed dimmed
and cannot be chosen), confirms the folder, shows each step as a spinner that ends ok, warn
or fail, folds what waits on an API into one line (`--verbose` lists it), and ends with the
next commands. `--template` and `--yes` skip the menu and the confirmation. Without a
terminal, or with `--json`, `NO_COLOR` or `TERM=dumb`, nothing is drawn or asked: the plain
`done:` and `not yet:` lines go to stderr, and stdout stays data only. The editor's own
question, Code asking before it opens the folder, is said once in the ending.

### The Planner plan

The coordinated template adds one step, "The project's Planner plan". `init` makes the
plan, or finds the one it made before (its `external_id` is `project:NAME`, so a second run
makes no second plan), and writes its `plan_...` id under `[plan]` in `coordination.toml`.

| Flag | What it does |
|---|---|
| `--plan-id PLAN` | Use this plan instead of making one. Signed in, init checks you can see it first; a plan it cannot find fails that step, and the message says the files are already written and how to put the id in by hand. Signed out, the id is written as given and said to be unchecked |
| `--no-plan` | Skip the Planner step; `[plan] id` stays empty. Not with `--plan-id` |

Signed out (no profile and no `IOHR_TOKEN`), offline or refused, init still works: the
Planner step is skipped, it prints how to set the id later (`iohr login`, then
`iohr coordinator plan create NAME --external-id project:NAME`, and put the id under
`[plan]`), and the exit code is 0. A session that exists but cannot be used (a token for
another audience, an expired one) is skipped the same way, and the message names the real
reason instead of "not signed in".

### Where the project keeps its data

By default the ADRs and notes (`memory/`) and the crawl live in the project's `.inorbit/`
and the machine state (the registry entry and caches) in `~/.inorbit/projects/NAME`. On the
first init at a terminal, `init` asks where all of it should go, and writes the answer as
`[project] data` in `inorbit.toml`:

| Answer | Notes, crawl | State |
|---|---|---|
| `project` (the default offered) | `<project>/.inorbit/memory`, `.inorbit/crawl` | `<project>/.inorbit/state`, `cache` (ignored by git through `.inorbit/.gitignore`) |
| `home` | `~/.inorbit/projects/NAME/memory`, `crawl` | `~/.inorbit/projects/NAME` |
| nothing recorded | `<project>/.inorbit/memory`, `.inorbit/crawl` | `~/.inorbit/projects/NAME` |

The third row is the layout every project had before this question existed, and it stays
as it is: no project changes until its owner chooses. The atlas stays in the project in
both places. The notes store, the crawl, the knowledge base and the project registry ask one
resolver (`project::place::Layout`), so they move together; the other writers (role notes,
handoff, permissions) still use the project's `.inorbit/` and follow in a later change.

| Flag | What it does |
|---|---|
| `--keep-in project\|home` | Answer the question. Also `IOHR_INIT_KEEP_IN`; the flag wins, then the variable, then what `inorbit.toml` records, then the question. With `--yes`, `--json` or no terminal and none of these, nothing is asked or recorded |
| `--copy-data` | When the place changes, copy what the old place holds into the new one. At a terminal it is offered instead. Nothing is ever deleted and nothing already in the new place is overwritten |

A dry run records and writes nothing and says what it would do.

### The project's shape

Init asks once whether the project is a monolith, microservices or combined, and files the
question as an item (`project-shape`, kind `question`) in the project's Planner plan, so
the answer is work the Mac app and the phone see as well. The answer is written as
`[project] shape` in `inorbit.toml` and as one fact in the project's notes, which `?` and
`??` read.

- At a terminal init asks inline, records the answer and marks the item done.
- `--shape monolith|microservices|combined` (or `IOHR_INIT_SHAPE`) answers without
  asking, with the same result.
- Without a terminal and without the flag the item stays open (`proposed`), for the Mac
  app or the phone to answer; nothing is recorded in the project yet.
- Signed out or offline, an answer is still kept in the project and the step says the item
  was not filed. Without a plan (`--no-plan`) there is nowhere to file it, and it says so.
- A dry run asks nothing.

## IO.md: instructions for the folder

An `IO.md` (or `io.md`) file holds what you want your agent to know about a folder, the way a
`CLAUDE.md` does: what the project is, how to build and test it, rules to keep. `?` and `??` load
the chain from the folder you are in up to the project root (the nearest folder with
`inorbit.toml`, `.inorbit/` or `atlas/`, else the git root: the one rule memory and the notes use;
never past your home folder), farthest first and the nearest last, so the
nearest file wins. One more file, `IO.md` or `io.md` in the iohr config directory
(`IOHR_CONFIG_DIR`), applies to every project and comes first. If a folder holds both spellings
it reads `IO.md` and warns once. Only a regular file counts: an `IO.md` or `io.md` that is a
symlink is skipped, so a link cannot pull in a file from elsewhere. Each file is cut at 4 KiB and all of them at 8 KiB (the nearest
files are kept first); the answer card's `Read:` footer says "context: IO.md x3" and, when
anything was cut or left out, how. `--json` `context.io_md` counts the files and bytes, never the
text. The text is yours: it shapes the answer but never widens what the agent may open or run,
and secrets in it are scrubbed before it is sent. `iohr context` prints the chain.

`iohr project init` writes a starter `IO.md` at the project root (what it is, the build and test
commands, the binary that ran init); `--no-io-md` skips it, and an `IO.md` or `io.md` you already
have is never overwritten. The loader is one module (`src/iomd.rs`); the project brief planned in
PRD 0193 is meant to join it as another kind of file rather than a second reader.

## Templates

```sh
iohr project template list        # name, status, origin, the default (*) and what it is
iohr project template list --json
iohr project template show demo   # what it asks, its languages, which files it writes, its steps
cd "$(iohr project template path my-sensor)"   # the folder of one of yours
```

The built-in templates are `coordinated`, `demo` and `blank`; `macos-app` and `ui-panel`
are `coming` and refuse until they exist. `iohr project templates` is the same as
`template list`, kept for older scripts.

Which template is used: `--template`, else `template` under `[project]` in `config.toml`
(`template = "demo"`; you edit it, `iohr config` does not write it), else `coordinated`. A
described project uses `blank`, whatever `[project]` says. A configured template that is
coming refuses too.

### Templates are data

A template is a folder with a `template.toml` and the files of the project. Nothing about
one is compiled in except the built-ins, which are the same folders embedded
([`templates/project/`](../crates/iohr/templates/project)).

```toml
id = "my-sensor"
description = "what it is"
status = "live"          # or "coming"
extends = "demo"         # optional: its files, questions and steps come first; yours replace by path
agent_prompt = "Make @@NAME@@ read @@KIND@@."   # optional, used by the agent step and --describe

[[questions]]
key = "kind"             # lowercase snake_case; the file text gets @@KIND@@
prompt = "Which kind?"
type = "choice"          # string, bool, choice, multi
choices = ["alpha", "beta"]
default = "alpha"
regex = "^[a-z]+$"       # string answers only
help = "shown above the question"
when = "kind == beta"    # also: `key`, `!key`, `key != value`; asked only when it holds

[[steps]]
name = "git"             # git, register, smoke, or agent (labelled AI)
```

Files are filled by the same `@@KEY@@` renderer as the built-ins (`@@NAME@@`, `@@ID@@`,
`@@MODULE@@`, `@@CREATED_AT@@` plus your question keys), and path segments are filled too
(`notes-@@KIND@@.md`). The shape follows copier's `copier.yml` (questions with a type,
choices, default and `when`; layering) and cargo-generate's `cargo-generate.toml`
(placeholders with a type, prompt, choices and regex).

The template comes from, in order: `--template PATH` (a folder), then
`~/.inorbit/templates/<id>/`, then the built-ins. `--template gh:owner/repo[//subdir][@ref]`
(or `git:URL`) shallow-clones a repository to read it, never in a dry run; `@ref` is a
branch or tag.

```sh
iohr project template new my-sensor [--extends demo]   # a working template in ~/.inorbit/templates
iohr project template lint ./my-sensor                 # checks keys, `when`, regexes, placeholders, a sample render
iohr project template schema                           # the JSON Schema of template.toml
iohr project template show my-sensor [--json]          # questions, `when`, files, the agent step
iohr project template path my-sensor                   # only the folder, for `cd $(...)`
```

Answers come from `--set key=value` (repeatable), then `--answers file.json|toml`, then, at
a terminal, the questions for what is still missing, then the defaults. `--yes` never asks
and takes the defaults, and a missing answer with no default is an error that names the
key.

### The `blank` template

`iohr project init my-tool --template blank --lang rust` writes the smallest project in your
language and nothing about InOrbit services: `README.md` (the name and a one-line
description from your words), `.gitignore`, `inorbit.toml`, `.inorbit/space`, and a
skeleton with one trivial test:

| Language | Skeleton |
|---|---|
| Rust | `Cargo.toml`, `src/main.rs` |
| Python | `pyproject.toml`, `src/<module>/__init__.py` |
| Go | `go.mod`, `main.go` |
| TypeScript | `package.json`, `src/index.ts` |

The smoke test is the language's own test command. Its kind in `inorbit.toml` is `project`.

## Describe it, your agent builds it

When `claude` or `codex` is on your `PATH` (or `[ask]` `agent` in `config.toml` names one),
the menu starts with "Describe it, your agent builds it", and `--describe "text"` does the
same. The agent works in the background: `init` draws the progress itself, runs to the end
and exits. There is no session to open or resume.

The order is fixed: scaffold, then the agent, then the smoke test, then the report.

1. **The scaffold** is the `blank` template (an empty project in your language;
   `--template` names another on purpose). Its `agent_prompt` says only "a fresh project in
   <language>; build what is described below; keep it free of secrets; keep `<test>`
   passing".
2. **The agent** is your own `claude` or `codex` in print mode, in the new folder, with a
   prompt from your words, the answers and the template. It may:
   - edit files;
   - run the language's own build and test commands (`cargo build|test|fmt|clippy`,
     `go build|test|vet`, the template's Python and Node tools);
   - run this project's own `atlas new`, `atlas sync --dry-run` and
     `memory remember|recall|list|check`, under the running binary's name;
   - run read-only `git status|log|diff` and `ls`;
   - fetch and search the web with WebFetch and WebSearch.

   Any other shell command (`curl` included) is denied and drawn dim with its reason. The
   prompt tells it to run one simple command per call and names the binary, the atlas and
   memory conventions, and that you send the result with `<bin> sync`. It carries the
   installed toolchain version and today's date, and tells the agent to use the latest
   stable, secure release and to check it at the upstream repository and its advisories.
   The `docs` language ("None, documents only") makes a project of atlas and memory with no
   program and no smoke test; its prompt has no code in it.

   The run is bounded by `--agent-turns` (default 150, or `[project] agent_turns`) and
   `--agent-timeout` (seconds, default 1800, or `[project] agent_timeout`), and runs on
   `--agent-model` (default `sonnet`, or `IOHR_INIT_AGENT_MODEL`, or
   `[project] agent_model`). The output says which values were used. The stream is the same
   as `iohr ask`: thinking dim, one row per tool, and a footer labelled "AI-built".
3. **The smoke test** then runs on what the agent built, so the report is true for the
   result. When it passes, what the agent built is committed on its own, in a second commit
   whose message says it is AI-built (`git log` shows both). When the smoke test fails or
   does not run, the tree is left as the agent left it, uncommitted, and the report says
   so.
4. **If the agent fails** or runs out of turns or time, `init` says so, keeps the scaffold,
   exits non-zero and prints one line to continue (`<bin> ask --allow-edits "continue the
   work"`). It also writes `.inorbit/handoff.md` (what was asked, the files written, the
   agent's last words); the next `ask --allow-edits` in that folder reads it into its
   prompt and removes it when that run succeeds.

`--json` passes the agent's own events through, as `iohr ask --json` does, then an
`iohr_summary` line (the agent, the bounds, how it ended) and the `iohr_report` line.
Prompts and answers are never logged or stored. With no agent, it says how to configure
one.

### Is the toolchain behind?

While you answer, `init` reads the installed Rust, Go, Node or Python version and asks that
language's own upstream for the latest stable:

| Language | Upstream |
|---|---|
| Rust | `static.rust-lang.org/dist/channel-rust-stable.toml` |
| Go | `go.dev/dl/?mode=json` |
| Node | `nodejs.org/dist/index.json` (the newest LTS line) |
| Python | `www.python.org/api/v2/downloads/release/` (the highest stable 3.x.y) |

The language menu shows it (`cargo found · rustc 1.90.0, 1.93.0 is out`), the dry run says
`would: report the toolchain check: ...`, and the report carries it with the command that
upgrades it (`rustup update stable`, `brew upgrade go` when Homebrew installed it,
`mise upgrade`, a download link, and so on).

- At a terminal, `init` asks `Upgrade Rust now with ...?` (default no) and runs the command
  only on yes. With `--yes`, without a terminal and in a dry run, it prints the command and
  never runs it.
- The whole lookup is bounded to about 3 seconds and runs beside the other steps. Offline,
  on an error, on a timeout or under `--json` it is skipped and the line says
  `could not check`; a version is never guessed and `init` never fails on it.
- `IOHR_TOOLCHAIN_UPSTREAM=off` turns the lookup off; any other value is a base URL for the
  routes `/rust`, `/go`, `/node` and `/python` (the tests use a local server).

## Reading your public site

```sh
iohr project init my-shop --crawl https://example.com --crawl-pages 50
iohr project init my-shop --crawl https://example.com --crawl-rate 2
iohr project init my-shop --crawl https://example.com --crawl-subdomains
iohr project init my-shop --no-crawl
```

`iohr project init --crawl https://example.com` reads that site on this machine, with the
`inorbit/agent` extension (`iohr agent atlas crawl`), and keeps what it found in the
project. Nothing is sent to InOrbit.

- **Consent.** At a terminal, a web address in the `--describe` text is asked about first
  (default no). `--crawl URL` is the consent, and with `--json`, `--yes` or no terminal
  nothing is read unless you pass it. `--no-crawl` never reads.
- **Limits.** Before it starts, init says what it will do: the host you gave only,
  robots.txt honoured, at most `--crawl-pages` pages (default 200) and `--crawl-minutes`
  minutes (default 5; the crawler's own caps are 20,000 pages and 240 minutes), 10 hosts,
  50 MB in all and 2 MB a page. `--dry-run` says the same and reads nothing.
- **Pace.** `--crawl-rate N` requests a second to one host (default 10, 1 to 50; the
  variable `IOHR_CRAWL_RATE` is the same, the flag wins; a value outside 1 to 50 is refused only when a crawl would run, `--no-crawl` never reads it), and a slower `Crawl-delay` in
  robots.txt wins over it. The text init shows before it starts states the rate.
- **Progress.** At a terminal the crawl shows a spinner with a live count (pages read, pages
  waiting) and, as soon as the crawler knows how much there is (a sitemap says so), a bar:
  pages done of the total, and how many the page limit still allows. The display updates at
  most once a second. It reads the crawler's `--progress` lines (RFC 0163: JSON on standard
  error, counts and the scope, never a page address). With `--json`, `NO_COLOR`,
  `TERM=dumb` or no terminal nothing is drawn. An `agent` extension from before RFC 0163
  does not know `--rate` or `--progress`: init runs it again without them and it crawls at
  its own pace with the spinner only.
- **Other names in the same domain.** The crawl stays on that host. When a page links to
  another name in the same domain (`blog.example.com` from `example.com`; from
  `www.example.com` that also means the apex and its siblings), init asks once per name at
  a terminal: yes, no, or yes to all, default no. A yes runs one more pass on that name
  alone, inside what is left of the same limits, and its raw record is in
  `.inorbit/crawl/<host>/more/<name>/`. With `--yes`, `--json` or no terminal, init follows
  none, and the summary lists the names it left out. `--crawl-subdomains` is the yes to all
  given in advance: the first pass is allowed the host and `*.host`, and nothing is asked.

What it keeps:

- `.inorbit/crawl/<host>/`: `crawl.json` (the raw record), `facts.jsonl` (one fact per page
  with the day it was crawled) and `policy.toml` (the policy it ran under). A `.gitignore`
  there keeps them out of the project's git.
- One local study, `atlas/studies/NNNN-site-crawl-<host>.md` (a second crawl rewrites it in
  place), which `iohr sync --dry-run` lists and `iohr sync` carries.
- One project memory entry that points at the study.

The summary says how many pages were read, how many robots.txt skipped and how many failed,
whether a limit stopped it (and the flag that raises it), and ends with "Nothing was sent to
InOrbit. Run iohr sync when you want it there." Without the `agent` extension, init says so
in one line and offers to install it at a terminal. A crawl that fails does not fail init:
it is a failed step with the extension's own error. The crawl day is the evidence time; the
crawler records no fetch time per page.

### The site knowledge base

The crawler keeps the HTML it read (`--keep-pages`, in `.inorbit/crawl/<host>/pages/`, one
file per page, a redirect followed to the page it names), and init reads those files: the
pages are not fetched a second time.

- **Older agents.** Only an agent that does not know `--keep-pages` leaves iohr to ask for
  the pages itself, as a fallback: inside the crawl's scope, only where `robots.txt` allows
  it, at the crawl rate or the site's `Crawl-delay` when that is slower, no redirect
  followed, 2 MB a page, within the same `--crawl-minutes`. The text init shows says how
  many pages came from files and how many were requested.
- **Empty shells.** A page whose HTML is an empty shell (a rendered page whose text only a
  browser run shows) is counted and named, not indexed.
- **The index.** The text is cleaned (main text, title, headings, links, language; blocks
  that repeat across the site are left out), cut along its headings into chunks of about
  180 words with a word count, a stable id (`ch_...`), its URL and character offsets, and
  indexed in one DuckDB file, `.inorbit/crawl/kb.duckdb`. Search is lexical (BM25 over a
  term table; heading and title words weigh more). There is no embedding model on this
  machine, so the summary and `--json` say `embeddings: stub` rather than pass a lexical
  score for a vector one.
- **The site model.** Entities (organizations, products, people, places, events,
  technologies, emails, phones, prices, profiles; from the page's own JSON-LD first, then
  patterns) and facts are lines in `.inorbit/crawl/<host>/facts.jsonl`, beside the page
  facts (`kind`, `site`, `url`, `crawled`, plus the chunk each came from). The sections,
  with their extractive summary, are a "Site model" part of the one "Site crawl" study.

`iohr atlas search QUERY`, `iohr atlas facts` and `iohr atlas index` read all of this
(`--site` picks a crawled site, `--json` gives data), and `?` and `??` add the best five
chunks for the question to what the model is given, as `<sources>` with each page URL and
chunk id (the page text is escaped, so a page cannot close the block or add instructions to
it), so an answer about the site can name where it came from. All of it stays on this
machine, gitignored, until you run `iohr sync`. A build that fails does not fail the crawl.

## The project record and the space link

Every template writes two committed files, neither holding a secret:

- **`inorbit.toml`** at the root is the project record `iohr` and InOrbit Desktop read. Its
  keys are grouped as the project (`name`, `id`, `kind`: `project`, `macos-service`,
  `macos-app` or `ui-panel`), where it came from (`template`, `template_version`,
  `created_at`) and the InOrbit links (`space_id`, `listing`, `memory_namespace`, each
  empty until the step that fills it exists; `iohr init` fills `space_id`). It was called
  `inorbit-project.toml`; nothing in the command line reads the record, so there is no
  fallback for the old name.
- **`.inorbit/space`** is the link to an InOrbit space: `id` and `name` as `key = "value"`
  lines, written by the scaffold step for every template. Until the space exists it holds
  empty values and a comment that says how to link it; `iohr init` and `iohr project sync`
  fill it. `.inorbit/` is committed, except `.inorbit/space.*` (the local environment's
  link), which its own `.gitignore` keeps out.

## The atlas and `iohr sync`

`iohr project init` also writes `atlas/` in the project root, a plain folder you can read
and edit:

- `prd/`, `rfc/`, `adr/` and `studies/`, with one starter each, numbered 0001 and filled
  from the project name, template, language and `--describe` text, and labelled as stubs;
- `canvas/` with a starter canvas (`iohr.canvas` version 3, the format InOrbit Desktop
  reads);
- `models/` for 3D files (glTF 2.0; a README, no fake model);
- `atlas/README.md`.

A second run never overwrites a file you have. `iohr atlas init` adds the folder to a
project made without it.

```sh
iohr atlas new rfc "Cache the catalogue"      # atlas/rfc/0002-cache-the-catalogue.md
iohr sync --dry-run                            # what would be created or saved; sends nothing
iohr sync                                      # send it to the space in .inorbit/space
```

A document is Markdown with front matter (`title`, `status`, `public`, `summary` and the
rest) that maps one to one onto the Decisions document. `iohr sync` uses the SDK's
Decisions client: it creates what is new, saves a new version of what changed on the
version it read, and on a conflict (the document changed in Decisions) re-reads, says so
and overwrites nothing. It writes `remote_id`, `remote_version`, `remote_number` and
`remote_sha` back into the front matter; those keys stay local. The canvas and the 3D
folder are never sent, and the output says so. No document text reaches a log.

Not built yet: syncing in the background. The `[sync]` `background` key in `config.toml` is
read (default `false`), but no background loop exists, so only `iohr sync` sends anything.

## Try it without changing anything

```sh
iohr project init my-shop --template demo --dry-run --yes
```

`--dry-run` (alias `--plan`) runs the whole flow, questions, defaults and `when` skips
included, and does nothing: no files or folders, no git, registration or smoke test, no
sign-in, no network call, and the agent is not started; its exact command and full prompt
are printed instead, still labelled AI. Every step reads `would: ...`, and it ends with
`dry run: nothing was changed`. With `--json` the plan is data: `files` (path and bytes),
`steps`, and `agent` (command and prompt). Every action goes through one gate that checks
it.

## The project in InOrbit: `iohr init` and `iohr project sync`

Signed in, `iohr init NAME` makes the project's space in InOrbit before it writes anything,
so the first commit holds the link: a Decisions space of kind `project`, named like the
folder. The team, and who can read it:

```sh
iohr init my-shop --team acme --visibility managers   # under the team acme; owners and admins read it
iohr init my-shop --visibility team                   # everyone in the account (the default)
iohr --env local init my-shop                         # the local environment, a link of its own
iohr project sync                                     # make or find the space later, from the folder
iohr init my-shop --no-sync                           # make nothing in InOrbit
```

At a terminal, a missing `--team` or `--visibility` is asked as a menu (a personal account
is "personal"; a team you are not in is refused before anything is made). The sync is
idempotent: the space is found by account and name, so a second run, or a teammate's
clone, never makes a second one. A clone whose sign-in cannot open the linked space is told
to ask a team owner or admin. The link is `.inorbit/space` for production (committed, and
`space_id` in `inorbit.toml` is filled) and `.inorbit/space.local` for `--env local`, kept
out of git by `.inorbit/.gitignore`. The project then shows under Projects in the app and
the console, and the team opens it from the link.

Init never fails for the sync. Not signed in, offline, or a server that is down or refuses
the call (401, 403, 408, 425, 429, 5xx) is a `queued` step with the answers kept in
`~/.inorbit/projects/NAME/space-sync.<env>.toml`; `iohr project sync` completes it and
clears it. Any other refusal is a failed step with the reason. `--dry-run` says `would:`
and calls nothing.

`--no-sync`, or no sign-in, also leaves the built-in roles in `atlas/roles/` with a plain
line saying so; `iohr sync` imports them later ([roles](roles-and-memory.md#built-in-roles)).

### Not built yet

The platform has no API yet for a space's members, per-project scopes and tokens, or the
project summary, so the chosen visibility is the space's access floor (the whole account or
its managers), not a member list. These steps, and the others that need InOrbit (the
project's private listing, a memory namespace, monitors and notifications), are reported
as `not yet`, each with the API it waits on. Nothing is skipped in silence, and a step that
needs your or an admin's say becomes an ask. Every step can be run again, and nothing is
overwritten.

After init, `iohr ext lint --preview`, `iohr ext dev` and `iohr ext dev --test` work in the
folder as for any extension ([building one](extensions.md#building-one)), and InOrbit
Desktop's Code opens it after asking you.
