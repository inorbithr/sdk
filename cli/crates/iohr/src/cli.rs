//! The command-line surface: every command, flag and environment variable.

use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use iohr_auth::ProfileName;

use inorbithr::DEFAULT_BASE_URL;

use crate::environments::{self, Environment, Environments};

/// The InOrbit command line.
///
/// Sign in, keep several accounts as profiles, and call the API. A token is never an
/// argument: pipe it to `iohr login --with-token`, or set `IOHR_TOKEN`.
#[derive(Debug, Parser)]
#[command(
    name = "iohr",
    bin_name = "iohr",
    version,
    about,
    long_about,
    propagate_version = true,
    max_term_width = 100,
    styles = crate::ui::clap_styles(),
    after_help = "Exit codes: 0 ok, 1 a finding or a failure, 2 usage, 3 not signed in, 4 forbidden."
)]
pub struct Cli {
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub command: Command,
}

/// Flags every command takes.
#[derive(Debug, Clone, Args)]
#[command(next_help_heading = "Global options")]
pub struct Global {
    /// The profile to use (default: the one `iohr profile use` chose).
    #[arg(long, global = true, env = "IOHR_PROFILE")]
    pub profile: Option<ProfileName>,
    /// Print JSON instead of tables.
    #[arg(long, global = true)]
    pub json: bool,
    /// Describe each call on stderr: method, path, status, time, request id.
    #[arg(short, long, global = true)]
    pub verbose: bool,
    /// The environment to call: `production` or `local`.
    ///
    /// Sets the API and sign-in addresses; `--base-url` and `IOHR_ISSUER` still win over
    /// it. The default is `production`.
    #[arg(long, global = true, env = "IOHR_ENV", value_parser = environments::parse_name, value_name = "NAME")]
    pub env: Option<String>,
    /// The API's address (default: the environment's).
    #[arg(long = "base-url", global = true, env = "IOHR_BASE_URL")]
    base_url: Option<String>,
    /// The sign-in service.
    #[arg(long, global = true, env = "IOHR_ISSUER", hide = true)]
    issuer: Option<String>,
    /// The OAuth client the command line signs in as.
    #[arg(long, global = true, env = "IOHR_CLIENT_ID", default_value = iohr_auth::DEFAULT_CLIENT_ID, hide = true)]
    pub client_id: String,
    /// Sign in as someone other than a person (`iohr runner` and `iohr coordinator` only).
    ///
    /// `io` is the `OAuth2` client `iohr-io` with the client credentials grant, so jobs Io
    /// files run on runners Io owns. `iohr runner` and `iohr coordinator` only;
    /// the secret is read from the file `IOHR_CLIENT_SECRET_FILE` names (mode 0600) or, on
    /// a Mac, from the keychain item the Mac bridge uses, never from an argument.
    #[arg(
        long,
        global = true,
        env = "IOHR_SIGN_IN",
        value_enum,
        value_name = "WHO"
    )]
    pub sign_in: Option<SignIn>,
    /// With `--sign-in io`: the role named in the `x-iohr-via` attribution header, such as
    /// `improver`. Attribution only, it grants nothing. Not `--via`, which `iohr ask` uses for
    /// where an answer comes from.
    #[arg(long, global = true, env = "IOHR_VIA_ROLE", value_name = "NAME")]
    pub via_role: Option<String>,
    /// The old name of `--via-role`, kept so scripts keep working. The header it sets is
    /// unchanged (`personality=...`).
    #[arg(
        long,
        global = true,
        env = "IOHR_VIA_PERSONALITY",
        value_name = "NAME",
        hide = true
    )]
    pub via_personality: Option<String>,
    /// With `--sign-in io`: the agent session named in `x-iohr-via`.
    #[arg(long, global = true, env = "IOHR_VIA_SESSION", value_name = "ID")]
    pub via_session: Option<String>,
    /// With `--sign-in io`: the coordinator job named in `x-iohr-via`.
    #[arg(long, global = true, env = "IOHR_VIA_JOB", value_name = "ID")]
    pub via_job: Option<String>,
    /// Where profiles are kept (default: the platform's config directory).
    #[arg(long, global = true, env = "IOHR_CONFIG_DIR", hide = true)]
    pub config_dir: Option<PathBuf>,
    /// Where extensions are kept (default: the platform's data directory).
    #[arg(long, global = true, env = "IOHR_DATA_DIR", hide = true)]
    pub data_dir: Option<PathBuf>,
}

/// The global flags that take a value, as they are typed (`--profile`, `-p`), read from the
/// definition above so a new one is never forgotten. Code that walks a raw command line
/// uses it to tell a flag's value from the command (`iohr --profile session session`).
#[must_use]
pub fn global_value_flags() -> Vec<OsString> {
    let command = Global::augment_args(clap::Command::new("iohr"));
    let mut flags = Vec::new();
    for arg in command.get_arguments() {
        if !arg.get_action().takes_values() {
            continue;
        }
        if let Some(long) = arg.get_long() {
            flags.push(OsString::from(format!("--{long}")));
        }
        if let Some(short) = arg.get_short() {
            flags.push(OsString::from(format!("-{short}")));
        }
    }
    flags
}

/// Who signs in when it is not a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SignIn {
    /// Io, the `OAuth2` client `iohr-io` (client credentials).
    Io,
}

impl Global {
    /// The role named for attribution: `--via-role` (`IOHR_VIA_ROLE`), else the hidden old
    /// name `--via-personality` (`IOHR_VIA_PERSONALITY`).
    #[must_use]
    pub fn via_role(&self) -> Option<&String> {
        self.via_role.as_ref().or(self.via_personality.as_ref())
    }

    /// The name of the environment this run targets: `--env`, `IOHR_ENV`, else the default.
    #[must_use]
    pub fn environment_name(&self) -> &str {
        self.env
            .as_deref()
            .unwrap_or_else(|| Environments::builtin().default_name())
    }

    /// The environment this run targets: `--env`, `IOHR_ENV`, else the default.
    #[must_use]
    pub fn environment(&self) -> &'static Environment {
        let list = Environments::builtin();
        let name = self.env.as_deref().unwrap_or_else(|| list.default_name());
        // The name was checked on the way in; the default is checked by the tests.
        list.get(name)
            .unwrap_or_else(|_| unreachable!("environment names are validated"))
    }

    /// The API's address: `--base-url` or `IOHR_BASE_URL`, else the environment's, else
    /// production's.
    #[must_use]
    pub fn base_url(&self) -> &str {
        self.base_url
            .as_deref()
            .or(self.environment().base_url.as_deref())
            .unwrap_or(DEFAULT_BASE_URL)
    }

    /// The sign-in service: `IOHR_ISSUER`, else the environment's, else production's.
    #[must_use]
    pub fn issuer(&self) -> &str {
        self.issuer
            .as_deref()
            .or(self.environment().issuer.as_deref())
            .unwrap_or(iohr_auth::DEFAULT_ISSUER)
    }

    /// Refuses an environment whose address is a placeholder, unless the caller named
    /// the API's address.
    ///
    /// # Errors
    ///
    /// The environment has no real address yet.
    pub fn check_environment(&self) -> Result<(), crate::Error> {
        if self.environment().placeholder && self.base_url.is_none() {
            return Err(crate::Error::Failed(format!(
                "the {} environment has no address yet; set IOHR_BASE_URL or --base-url",
                self.env.as_deref().unwrap_or("selected")
            )));
        }
        Ok(())
    }
}

/// The top-level commands by heading, the way `gh` groups its own. clap 4 lists subcommands
/// under one heading only (`Command::subcommand_help_heading`), so [`command`] writes the
/// root's `help_template` from this table. A test holds every visible command to exactly one
/// group, so a new command cannot be left out of `iohr --help`.
pub const GROUPS: &[(&str, &[&str])] = &[
    (
        "Sign-in",
        &["login", "logout", "whoami", "profile", "auth", "token"],
    ),
    (
        "Account",
        &["accounts", "domains", "connectors", "connections", "config"],
    ),
    (
        "Build",
        &[
            "init",
            "project",
            "atlas",
            "sync",
            "ext",
            "sdk",
            "api",
            "openapi",
            "lab",
            "decisions",
        ],
    ),
    (
        "Agents",
        &[
            "ask",
            "asks",
            "permissions",
            "shell",
            "runner",
            "session",
            "stop",
            "coordinator",
            "memory",
        ],
    ),
    (
        "Operate",
        &["tunnel", "browser", "upgrade", "completion", "help"],
    ),
];

/// The command tree `main` parses with: [`Cli`]'s, with the root help grouped by
/// [`GROUPS`] and colour off under `--json` (a pipe and `NO_COLOR` are clap's own rules).
///
/// clap's `help_template` accepts runtime text
/// (<https://docs.rs/clap/4.6/clap/struct.Command.html#method.help_template>); `{all-args}`
/// still renders Options and Global options with their headings, so the subcommands are
/// hidden from it and listed here instead. Hidden commands still run.
#[must_use]
pub fn command(json: bool) -> clap::Command {
    use clap::CommandFactory as _;
    use std::fmt::Write as _;
    let mut cmd = Cli::command();
    if json {
        cmd = cmd.color(clap::ColorChoice::Never);
    }
    let styles = cmd.get_styles().clone();
    let (head, lit) = (styles.get_header(), styles.get_literal());
    // The examples every command ends its help with: the same table the usage errors read.
    let paint = |style: &clap::builder::styling::Style, text: &str| {
        format!("{}{text}{}", style.render(), style.render_reset())
    };
    for (path, list) in crate::examples::ALL {
        if path.is_empty() {
            continue;
        }
        let text = crate::examples::block(
            list,
            &crate::examples::Style {
                head: &|t| paint(head, t),
                command: &|t| paint(lit, t),
                note: &str::to_owned,
            },
        );
        cmd = with_after_help(cmd, path, &text);
    }
    let listed: Vec<(String, String)> = cmd
        .get_subcommands()
        .filter(|c| !c.is_hide_set())
        .map(|c| {
            (
                c.get_name().to_owned(),
                c.get_about().map(ToString::to_string).unwrap_or_default(),
            )
        })
        .collect();
    let width = listed.iter().map(|(n, _)| n.len()).max().unwrap_or(0) + 2;
    let mut groups = String::new();
    for (title, names) in GROUPS {
        let _ = writeln!(groups, "{}{title}:{}", head.render(), head.render_reset());
        for name in *names {
            let about = listed
                .iter()
                .find(|(n, _)| n == name)
                .map_or("Print this message or the help of a command", |(_, a)| {
                    a.as_str()
                });
            let _ = writeln!(
                groups,
                "  {}{name}{}{:pad$}{about}",
                lit.render(),
                lit.render_reset(),
                "",
                pad = width - name.len()
            );
        }
        groups.push('\n');
    }
    let template = format!(
        "{{about-with-newline}}\n{{usage-heading}} {{usage}}\n\n{groups}{{all-args}}{{after-help}}"
    );
    let names: Vec<String> = listed.into_iter().map(|(n, _)| n).collect();
    for n in names {
        cmd = cmd.mut_subcommand(n, |c| c.hide(true));
    }
    cmd.help_template(template)
}

/// `text` as the `after_help` of the command at `path` (a subcommand's, however deep).
fn with_after_help(cmd: clap::Command, path: &[&str], text: &str) -> clap::Command {
    match path.split_first() {
        None => cmd.after_help(text.to_owned()),
        Some((first, rest)) => cmd.mut_subcommand(first, |c| with_after_help(c, rest, text)),
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Sign in and add a profile.
    ///
    /// Sign in and add a profile: in a browser when this machine can open one, with
    /// a device code otherwise.
    Login(Login),
    /// Forget a profile and its credential on this machine.
    Logout,
    /// The profiles on this machine.
    #[command(subcommand)]
    Profile(ProfileCommand),
    /// Who the active profile is: subject, account, scopes, expiry.
    Whoami,
    /// Credentials for other programs: a profile's current access token.
    #[command(subcommand)]
    Auth(AuthCommand),
    /// The accounts the credential can see.
    #[command(subcommand)]
    Accounts(AccountsCommand),
    /// API tokens for an account.
    #[command(subcommand)]
    Token(TokenCommand),
    /// Make one authenticated call and print the answer.
    Api(ApiCall),
    /// The OpenAPI document for this credential.
    #[command(subcommand)]
    Openapi(OpenapiCommand),
    /// Generate an SDK cut to what your credentials may call, and check it later.
    #[command(subcommand)]
    Sdk(SdkCommand),
    /// Domains an account controls, proved with one DNS record.
    #[command(subcommand)]
    Domains(DomainsCommand),
    /// List and file the coordinator's jobs and asks.
    ///
    /// The coordinator's jobs and queued asks: list and file work.
    #[command(subcommand)]
    Coordinator(CoordinatorCommand),
    /// Read and answer the asks waiting for you: list, show, deny, question.
    ///
    /// The asks agents file for you (a pull request to merge, a roll, a review). `list` and
    /// `show` stay live on a terminal and print once with `--once`, `--json` or in a pipe.
    /// You can deny an ask or put a question to its agent here; approving needs your phone's
    /// key, so `approve` says so and waits for it.
    #[command(subcommand)]
    Asks(AsksCommand),
    /// Keep what a project knows as files, check it, and sync it when you choose.
    ///
    /// Memory is local first: one Markdown file per memory in `.inorbit/memory/` (committed with
    /// the code) or in your own notes, with no sign-in and no network. `check` finds what is
    /// wrong, `drift` what moved in the code, and `sync` shares it with your InOrbit account
    /// only when you turn that on in `[memory]` or run it.
    #[command(subcommand)]
    Memory(MemoryCommand),
    /// Browse the apps a connection can be made from.
    ///
    /// The apps a connection can be made from: incident tools, chat, observability, AI models.
    #[command(subcommand)]
    Connectors(ConnectorsCommand),
    /// Manage the account's connections to outside systems.
    ///
    /// The account's connections to outside systems: add one with a key or by signing
    /// in, test, pause, grant its actions, delete it.
    #[command(subcommand)]
    Connections(ConnectionsCommand),
    /// Install, verify and run extensions.
    ///
    /// Extensions: separate programs iohr installs from a registry, verifies, pins and
    /// runs as `iohr <name> ...`.
    #[command(subcommand)]
    Ext(ExtCommand),
    /// Every package you have in one table.
    ///
    /// Every package you have in one table: published by your account (public or
    /// private), private ones shared with it, unpublished ones in progress on this
    /// machine, and what is installed. The same as `iohr ext packages`.
    #[command(hide = true)]
    Packages,
    /// Update iohr to the newest release.
    ///
    /// Update this `iohr` to the newest release, the way it was installed: Homebrew runs
    /// `brew upgrade inorbithr/tap/iohr` (and says `brew trust inorbithr/tap` when
    /// Homebrew refuses the tap); a copy from the install script is downloaded, checked
    /// against the release's checksums and attestations and replaced in place; anything
    /// else gets the command to run. `--check` only reports. Needs no sign-in.
    Upgrade {
        /// Only say whether a newer release exists and what would run; change nothing.
        #[arg(long)]
        check: bool,
    },
    /// Settings besides profiles: where extensions come from, which keys may sign them.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Check lab documents kept in your repository.
    ///
    /// Lab documents: RFCs and studies kept in your repository.
    #[command(subcommand)]
    Lab(LabCommand),
    /// Read and write RFCs, PRDs and ADRs in Decisions.
    ///
    /// Decisions: RFCs, PRDs and ADRs in the Decisions product:
    /// list and read them, create one, save a new version, set its status, comment, link a
    /// pull request, ask for a review.
    #[command(subcommand)]
    Decisions(DecisionsCommand),
    /// Create projects and manage project templates.
    ///
    /// A project you build yourself: one command sets up everything it needs,
    /// from sign-in to a smoke test, and reports what you now have.
    #[command(subcommand)]
    Project(ProjectCommand),
    /// Create a project: the same as `iohr project init`.
    ///
    /// Sets up a project of your own and creates it in InOrbit under your team, for the
    /// environment `--env` names, linked in `.inorbit/space`. See `iohr project init --help`.
    Init(Box<InitArgs>),
    /// Keep the project's PRDs, RFCs, ADRs and studies in its atlas/ folder.
    ///
    /// The atlas: plain files in the project's `atlas/` folder (PRDs, RFCs, ADRs and studies as
    /// Markdown, a canvas, a folder for 3D models). They stay on your machine until you run
    /// `iohr sync`.
    #[command(subcommand)]
    Atlas(AtlasCommand),
    /// Send the project's atlas documents to its Decisions space.
    ///
    /// The same as `iohr atlas sync`: creates what is new, saves a new version of what changed,
    /// and never overwrites a document that changed in Decisions. Nothing else leaves
    /// the machine.
    Sync(SyncArgs),
    /// Ask your own AI agent a question in this folder.
    ///
    /// Ask a question in this folder: your own AI agent (Claude Code, or Codex) answers once,
    /// right here in the terminal, under your own subscription. No session opens: the agent
    /// runs in print mode with the small fast model (`haiku`, or `--model`, `IOHR_ASK_MODEL`, `[ask] model`) and iohr draws what it thinks
    /// (dim, in a quiet block), each tool it uses (one row each) and the answer as it arrives,
    /// then a footer with the time, turns and cost. Without a terminal, with `NO_COLOR` or
    /// TERM=dumb it prints only the answer; `--json` passes the agent's own events through as
    /// JSON lines. Nothing is changed unless you pass `--allow-edits` (files only); shell
    /// commands are never allowed. iohr never stores the question or the answer. Defaults
    /// come from `[ask]` in config.toml; the flags win for one call.
    Ask(Box<AskArgs>),
    /// Shell integration: `? question` at your prompt asks your AI agent.
    ///
    /// `iohr shell hook zsh` prints what makes `? question` at your prompt ask your own AI
    /// agent. Add `eval "$(iohr shell hook zsh)"` to ~/.zshrc.
    #[command(subcommand)]
    Shell(ShellCommand),
    /// Rules for what Io may run or open when you ask it something.
    ///
    /// `iohr ask` (and `?` at your prompt) asks you before Io runs a command. Your answers
    /// are rules in Claude Code's own grammar (`Bash(ls:*)`, `Read(./src/**)`,
    /// `WebFetch(domain:example.com)`), kept in `.inorbit/permissions.toml` in the project
    /// and in your config folder. A deny rule beats an allow rule, and keys, `.env` files
    /// and credentials are never allowed by any rule or answer. Every decision is appended
    /// to `.inorbit/permissions.log` (the tool and the rule, never output or file contents).
    #[command(subcommand)]
    Permissions(PermissionsCommand),
    /// Serves the permission questions of one `iohr ask` run to Claude Code (not for you).
    #[command(name = "permission-server", hide = true)]
    PermissionServer {
        /// The private socket of the `iohr ask` that started it.
        #[arg(long, value_name = "PATH")]
        socket: std::path::PathBuf,
    },
    /// Run coordinator turns on this machine.
    ///
    /// The coordinator's runner: this machine runs coordinator turns on your own
    /// installed `claude`, on your own subscription.
    #[command(subcommand)]
    Runner(RunnerCommand),
    /// Open this project's session: one process hosts its Roles while iohr is open.
    ///
    /// Joins this project's session host (RFC 0213), starting it when none runs: one
    /// `iohr runner run --session DIR` process per project that hosts its Roles and works from
    /// a durable queue (a `DuckDB` file in iohr's data directory, so nothing is lost when it
    /// stops or the machine restarts). The command stays open as the session and prints
    /// nothing on stdout; Ctrl-C, SIGTERM or the terminal closing leaves it. The host stops
    /// when the last session has left, or on `iohr stop`.
    ///
    /// Slice S1 of RFC 0213: no tasks are accepted from the server yet, and a task a Role
    /// claims fails with "not built". The host, its Roles, the durable queue and the session
    /// link work; `iohr ask --work` and the real handler come next.
    Session(SessionArgs),
    /// Stop this project's session host.
    ///
    /// Tells the project's session host to stop. Each Role finishes the task it holds, then
    /// the host ends; `--wait` (15 seconds by default) waits until it has, and says
    /// `stopping` when it has not yet. A task cut short by a crash instead is taken again by
    /// the next host as soon as it starts.
    Stop(StopArgs),
    /// Install the InOrbit Trails browser extension.
    ///
    /// InOrbit Trails, the browser extension: open the page that adds it to your browser.
    #[command(subcommand)]
    Browser(BrowserCommand),
    /// Open a loopback pipe to an environment's edge.
    ///
    /// A loopback pipe from this machine into an environment's Envoy edge, so the real
    /// site opens at `http://www.localhost:<port>/` (and console, docs, auth). Runs
    /// `ssh -N -L` until Ctrl-C; binds 127.0.0.1 only and goes through Envoy, never to a
    /// service directly. The environment is `--env` (production by default).
    Tunnel(Tunnel),
    /// Print a shell completion script.
    Completion {
        /// The shell.
        shell: Shell,
    },
    /// An installed extension, such as `iohr agent ...`.
    #[command(external_subcommand)]
    External(Vec<OsString>),
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Print a profile's access token.
    ///
    /// Print the profile's access token, refreshing a signed-in session first when less
    /// than a minute of it is left.
    ///
    /// For programs that call the API with your login, such as the SDKs' `cli`
    /// credential source (`docs/config.md` section 5.4). The refresh token never leaves
    /// the credential store. Exit codes: 0 printed, 2 usage, 3 not signed in (no such
    /// profile, no credential, an expired token or an ended session), 1 anything else,
    /// such as the sign-in service being unreachable.
    Token(AuthToken),
}

#[derive(Debug, Args)]
pub struct AuthToken {
    /// `text`: the token alone on one line. `json`: one line with `access_token`,
    /// `expires_at`, `profile` (null
    /// for `IOHR_TOKEN`) and `account`. `--json` is the same as `--format json`.
    #[arg(long, value_enum, default_value_t = TokenFormat::Text)]
    pub format: TokenFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TokenFormat {
    Text,
    Json,
}

#[derive(Debug, Subcommand)]
pub enum LabCommand {
    /// Check RFCs and studies against the lab's rules.
    ///
    /// Check RFCs and studies the way the InOrbit site checks its own: file names,
    /// front matter, status logs, and the redaction rules on public documents. Reads
    /// only the files named, makes no network call, needs no sign-in.
    Check(LabCheck),
}

#[derive(Debug, Args)]
pub struct LabCheck {
    /// Folders of documents (`studies` holds studies, any other folder RFCs), or single
    /// files. Default: `docs/rfcs` and `docs/studies`, those that exist.
    pub paths: Vec<PathBuf>,
    /// The lab's own redaction config: its domains, words and patterns. Default:
    /// `docs/lab/redaction.json` when it exists, otherwise the generic rules alone.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Strict rules to add, in the rules file shape (`{"rules": [...]}`), such as the
    /// platform's `docs/lab/strict.json`. Checked on public documents like the others;
    /// a hit says the rule and why, never what matched. Not read unless named.
    #[arg(long, value_name = "FILE")]
    pub strict: Option<PathBuf>,
}

/// Where a document is looked for in Decisions.
#[derive(Debug, Clone, Args)]
pub struct DecisionsWhere {
    /// The space, by slug (`platform`) or id (`lab_...`). Default: every space of the
    /// account; needed when two spaces hold the same number.
    #[arg(long)]
    pub space: Option<String>,
    /// The account whose spaces to use. Default: the credential's own.
    #[arg(long)]
    pub account: Option<String>,
    /// The kind of document: `rfc` (default), `prd`, `adr` or `study`; `all`
    /// means every kind. A number names a document of this kind; `PRD 0001` or
    /// `ADR 0003` in place of a number names its kind itself.
    #[arg(long, default_value = "rfc", value_parser = ["rfc", "prd", "adr", "study", "all"])]
    pub kind: String,
}

/// A pull request a comment is about.
#[derive(Debug, Clone, Args)]
pub struct PrLine {
    /// The pull request: `repo#n` (an inorbithr repository), `owner/repo#n` or its
    /// GitHub link.
    #[arg(long)]
    pub pr: Option<String>,
    /// Its title.
    #[arg(long, requires = "pr")]
    pub title: Option<String>,
    /// It was merged.
    #[arg(long, requires = "pr", conflicts_with = "live")]
    pub merged: bool,
    /// The change is live: merged, rolled and serving.
    #[arg(long, requires = "pr")]
    pub live: bool,
}

#[derive(Debug, Subcommand)]
pub enum RunnerCommand {
    /// Register this machine as a runner.
    ///
    /// Register this machine as one of your runners: its name, system and what it can run.
    ///
    /// Needs `claude` on your PATH, signed in with your subscription (`claude`, then
    /// `/login`). Nothing about the sign-in leaves the machine except a hash of the seat
    /// `claude auth status` names (organisation and email). Run it again and it says the runner exists; with
    /// `--refresh` it sends the registration again with the key this machine holds, so
    /// the runner keeps its id and the coordinator records its current sign-in.
    Register {
        /// The runner's name (default: `[runner] name` in config.toml, else this
        /// machine's host name).
        #[arg(long)]
        name: Option<String>,
        /// Turns this runner may hold at once, 1 to 32 (default: `[runner] capacity`,
        /// else 1). `iohr runner run` declares the configured capacity on every
        /// heartbeat, so a later change needs no new registration.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=32))]
        capacity: Option<u32>,
        /// Mark the runner as always on (a server), so the coordinator can rely on it
        /// (default: `[runner] always_on`).
        #[arg(long)]
        always_on: bool,
        /// An agent's runner only (with `--sign-in io`): the subject of the person whose
        /// provider sign-in it holds, the agent's seat holder. That person must already
        /// hold the same sign-in on a runner of their own, and the runner then takes only
        /// their verified work.
        #[arg(long, value_name = "SUBJECT", value_parser = clap::builder::NonEmptyStringValueParser::new())]
        holder: Option<String>,
        /// Register again a runner this machine already holds the key of: its name,
        /// system, capacity and the hash of its current provider sign-in are sent anew
        /// (after signing in to another account, or to update a registration).
        #[arg(long)]
        refresh: bool,
        #[command(flatten)]
        account: AccountArg,
    },
    /// Run the runner loop until Ctrl-C.
    ///
    /// Run the loop: heartbeat, accept a lease, run its turn on `claude -p`, report it.
    /// Stops on Ctrl-C.
    Run {
        /// The runner's name (default: this machine's host name).
        #[arg(long, conflicts_with = "runner")]
        name: Option<String>,
        /// The runner's id, from `iohr runner status` (or `id` in `[profiles.<name>.runner]`).
        #[arg(long, env = "IOHR_RUNNER_ID")]
        runner: Option<String>,
        /// Seconds between heartbeats (default: what the service says, the RFC's 20).
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=300))]
        interval: Option<u64>,
        /// Stop after this many answered heartbeats; for scripts.
        #[arg(long)]
        max_beats: Option<u64>,
        /// Turns to run at once, 1 to 32, declared on every heartbeat (default:
        /// `[runner] capacity` in config.toml, read again on SIGHUP; else what the
        /// runner registered with).
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=32))]
        capacity: Option<u32>,
        /// The turn classes to take, comma separated: `coordinate`, `build` (default:
        /// `[runner] kinds`, else both). Build turns pause while /mnt/build has under
        /// 100 GB free.
        #[arg(long, value_delimiter = ',')]
        kinds: Option<Vec<String>>,
        /// Never look for a newer iohr or restart into one (default: `[runner]
        /// auto_upgrade` in config.toml, which is on). With it on, the runner checks
        /// about hourly, drains, installs the newest release through `iohr upgrade`'s
        /// verified path and restarts itself.
        #[arg(long)]
        no_auto_upgrade: bool,
        /// Run as a project's session host (RFC 0213): the runner hosts the project's
        /// Roles beside its turns, never upgrades itself, and stops when the last session
        /// has left, on `iohr stop`, on SIGTERM or on Ctrl-C. `iohr session` starts it.
        #[arg(long, value_name = "DIR")]
        session: Option<PathBuf>,
        #[command(flatten)]
        account: AccountArg,
    },
    /// The runner's settings from config.toml.
    ///
    /// The runner's settings from config.toml: `[runner]` and `[profiles.<name>.runner]`.
    #[command(subcommand)]
    Config(RunnerConfigCommand),
    /// Run the check on a job's checkout and send a signed report.
    ///
    /// Run the check on a build job's checkout and send the service a signed report of it.
    ///
    /// Fetches `origin/main`, refuses a checkout with uncommitted changes or one that is
    /// not merged with `origin/main`, runs the command with a scrubbed environment, and
    /// signs the command, its exit code, the head, main's commit and the SHA-256 of the
    /// output (the output stays here) with this machine's runner key. The service
    /// checks the signature against the key you enrolled and the lease you hold; a report
    /// that fails the check holds the job at Checked, which is the right answer.
    GateReport {
        /// The job (`job_...`) whose lease you hold.
        #[arg(long)]
        job: String,
        /// Its lease (`lse_...`).
        #[arg(long)]
        lease: String,
        /// The lease's fencing token.
        #[arg(long)]
        token: i64,
        /// The checkout of the job's branch (default: the current directory).
        #[arg(long, default_value = ".")]
        repo: std::path::PathBuf,
        /// The check to run, through the shell (default: `mise run ci:changed`).
        #[arg(long, default_value = crate::runner::gate::DEFAULT_COMMAND)]
        command: String,
        /// Seconds the check may run (default 1800).
        #[arg(long, default_value_t = crate::runner::gate::DEFAULT_TIMEOUT_SECS, value_parser = clap::value_parser!(u64).range(1..=21600))]
        timeout: u64,
        /// The runner's name (default: the one whose key this machine holds).
        #[arg(long, conflicts_with = "runner")]
        name: Option<String>,
        /// The runner's id, from `iohr runner status` (or `id` in `[profiles.<name>.runner]`).
        #[arg(long, env = "IOHR_RUNNER_ID")]
        runner: Option<String>,
        #[command(flatten)]
        account: AccountArg,
    },
    /// Your runners as the service sees them, and what this machine can run.
    Status {
        #[command(flatten)]
        account: AccountArg,
    },
    /// Show or enrol this machine's run key.
    ///
    /// This machine's run key: the P-256 key that signs what it reports on approved
    /// pinned runs, and whether the approvals service has it enrolled.
    ///
    /// Makes the key the first time and keeps it in the credential store beside the
    /// runner key. Not enrolled yet, it asks you on your phone (a `runner_enrollment`
    /// ask showing the key's label and fingerprint), waits up to `--wait` minutes (15)
    /// for your approval and enrols the key; run it again to keep waiting on the same ask.
    /// `--proof` still enrols from a file holding your device's proof (`key_id`,
    /// `signed_at`, `nonce`, `signature`) over the enrolment message.
    RunKey {
        /// A JSON file with your device's proof over the enrolment message.
        #[arg(long)]
        proof: Option<std::path::PathBuf>,
        /// The enrolment's label, up to 100 characters (default: `iohr runner on` and
        /// this machine's host name).
        #[arg(long)]
        label: Option<String>,
        /// Minutes to wait for your approval, 0 to 60; 0 files the ask and returns.
        #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u64).range(0..=60))]
        wait: u64,
        /// Only show this machine's run key and its enrolled runner id; make nothing,
        /// ask nothing.
        #[arg(long, conflicts_with_all = ["proof", "label"])]
        show: bool,
        #[command(flatten)]
        account: AccountArg,
    },
}

#[derive(Debug, Subcommand)]
pub enum RunnerConfigCommand {
    /// Show the values the runner would use now.
    ///
    /// The values `iohr runner run` would use now, each with where it came from: a flag,
    /// `[profiles.<name>.runner]`, `[runner]` or the default. Needs no sign-in.
    Show {
        /// As `iohr runner run --capacity`.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=32))]
        capacity: Option<u32>,
        /// As `iohr runner run --kinds`.
        #[arg(long, value_delimiter = ',')]
        kinds: Option<Vec<String>>,
        /// As `iohr runner run --no-auto-upgrade`.
        #[arg(long)]
        no_auto_upgrade: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum DecisionsCommand {
    /// The documents of the account's spaces: number, space, status, access, title.
    List {
        #[command(flatten)]
        at: DecisionsWhere,
        /// Only documents with this status: an RFC's `draft`, `in_review`, `accepted`,
        /// `rejected`, `superseded` or `withdrawn`; an ADR's `proposed`, `accepted`,
        /// `rejected` or `superseded` (`open` and `decided` still read as before).
        #[arg(long)]
        status: Option<String>,
        /// Only documents whose title, slug or text contains this, any case.
        #[arg(long)]
        query: Option<String>,
        /// Only yours: documents you own, write or build, or are asked to review.
        #[arg(long)]
        mine: bool,
    },
    /// One document: its number, status, version and, with `--text`, its text.
    Show {
        /// The document: its number (`0065`, a part `0065.1`) or its id (`ldoc_...`).
        rfc: String,
        #[command(flatten)]
        at: DecisionsWhere,
        /// Print the document's text after its details.
        #[arg(long)]
        text: bool,
        /// With `--text`: only the text, as saved, so `> file.md` gives the file back.
        #[arg(long, requires = "text")]
        raw: bool,
        /// An older version instead of the current one.
        #[arg(long = "at-version", value_name = "N")]
        at_version: Option<u32>,
    },
    /// Create an RFC, PRD or ADR in a space.
    ///
    /// A new RFC, PRD or ADR in a space: the account gives it the next number of its
    /// kind. With `--file`, the file's text is saved as its next version straight away.
    Create {
        /// The space, by slug (`platform`) or id.
        #[arg(long)]
        space: String,
        /// The kind: `rfc` (default), `prd` or `adr`.
        #[arg(long, default_value = "rfc", value_parser = ["rfc", "prd", "adr"])]
        kind: String,
        /// The account the space belongs to. Default: the credential's own.
        #[arg(long)]
        account: Option<String>,
        /// The title, 1 to 200 characters.
        #[arg(long)]
        title: String,
        /// A one-paragraph summary, at most 1000 characters.
        #[arg(long)]
        summary: Option<String>,
        /// Make it a part of this document (`0065` makes `0065.N`).
        #[arg(long)]
        parent: Option<String>,
        /// The whole document to save after creating it (`-` reads stdin).
        #[arg(long)]
        file: Option<PathBuf>,
        /// What the saved version says changed (with `--file`).
        #[arg(long, short, requires = "file")]
        message: Option<String>,
    },
    /// Save a new version of a document from a file.
    ///
    /// Save a new version of a document from a file: the whole document, front matter and
    /// all. Refused when someone saved since the version it was made from.
    Save {
        /// The document: its number or id.
        rfc: String,
        #[command(flatten)]
        at: DecisionsWhere,
        /// The whole document (`-` reads stdin), at most 512 KiB.
        #[arg(long)]
        file: PathBuf,
        /// What changed, at most 500 characters.
        #[arg(long, short)]
        message: String,
        /// The version the file was made from. Default: the current one.
        #[arg(long)]
        base: Option<u32>,
    },
    /// Set a document's status.
    ///
    /// Set a document's status in the words of its kind: an RFC's `draft`,
    /// `in_review`, `accepted`, `rejected`, `withdrawn` or `superseded` (with
    /// `--successor`); a PRD's the same without `withdrawn`; an ADR's `proposed`,
    /// `accepted`, `rejected` or `superseded`. `open` and `decided` still read as before.
    Status {
        /// The document: its number or id.
        rfc: String,
        /// The new status.
        status: String,
        #[command(flatten)]
        at: DecisionsWhere,
        /// For `superseded`: the document that replaces it, in the same space.
        #[arg(long)]
        successor: Option<String>,
    },
    /// Comment on a document.
    ///
    /// Comment on a document. With `--pr`, the comment starts with the pull request's line
    /// (`owner/repo#n "title": open`, `merged` or `live`), so the document's thread reads
    /// as the record of the work; the same line `mise run decisions:comment` writes.
    Comment {
        /// The document: its number or id.
        rfc: String,
        /// The comment, 1 byte to 10 KiB with the pull request's line.
        text: String,
        #[command(flatten)]
        at: DecisionsWhere,
        #[command(flatten)]
        pr: PrLine,
        /// The heading the comment is on, by its id (`status-log`).
        #[arg(long)]
        anchor: Option<String>,
    },
    /// Record a pull request that implements a document.
    ///
    /// Record a pull request that implements a document, as a comment with its line alone:
    /// `owner/repo#n "title": open` when it opens, then `merged`, then `live`.
    LinkPr {
        /// The document: its number or id.
        rfc: String,
        /// The pull request: `repo#n` (an inorbithr repository), `owner/repo#n` or its
        /// GitHub link.
        pr: String,
        #[command(flatten)]
        at: DecisionsWhere,
        /// The pull request's title.
        #[arg(long)]
        title: Option<String>,
        /// The pull request was merged.
        #[arg(long, conflicts_with = "live")]
        merged: bool,
        /// The change is live: merged, rolled and serving.
        #[arg(long)]
        live: bool,
    },
    /// Create a diagram, or save a new version of one.
    ///
    /// Create a diagram from a model file, or with its id save a new version of one. The
    /// model is the Decisions product's diagram JSON (`version`, `nodes`, `edges`); embed it in
    /// a document with a ```` ```diagram <id> ```` block.
    Diagram {
        /// The diagram to save a new version of (`ldia_...`); without it, a new diagram.
        id: Option<String>,
        #[command(flatten)]
        at: DecisionsWhere,
        /// The model, JSON (`-` reads stdin), at most 1 MiB.
        #[arg(long)]
        file: PathBuf,
        /// Its name, 1 to 80 characters: needed for a new diagram; renames an existing one.
        #[arg(long)]
        name: Option<String>,
        /// What changed (for a new version), at most 500 characters.
        #[arg(long, short)]
        message: Option<String>,
    },
    /// Ask for a review of a document.
    ///
    /// Ask for a review of a document's current version from members of its account who
    /// may write (never yourself or the document's author).
    Review {
        /// The document: its number or id.
        rfc: String,
        #[command(flatten)]
        at: DecisionsWhere,
        /// A reviewer's subject; repeat, or separate with commas, for several.
        /// `mise run decisions:token` sets `IOHR_DECISIONS_REVIEWER` to the account's owner.
        #[arg(
            long = "reviewer",
            value_name = "SUBJECT",
            env = "IOHR_DECISIONS_REVIEWER",
            value_delimiter = ',',
            required = true
        )]
        reviewers: Vec<String>,
        /// Why, or what to look at: added as a comment with the request.
        #[arg(long)]
        reason: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum DomainsCommand {
    /// Start proving a domain: prints the TXT record to add.
    Add {
        /// The domain, such as acme.hr.
        domain: String,
        /// Prove only this subdomain's subtree, not its parent.
        #[arg(long)]
        subdomain: bool,
        /// The account (default: the profile's).
        #[arg(long)]
        account: Option<String>,
    },
    /// Look the record up now, from several public resolvers.
    Verify {
        /// The domain.
        domain: String,
        /// Keep checking every 10 seconds until the record is seen.
        #[arg(long)]
        wait: bool,
        /// With --wait, give up after this many seconds.
        #[arg(long, default_value_t = 600, value_parser = clap::value_parser!(u64).range(10..=3600))]
        timeout: u64,
        /// The account (default: the profile's).
        #[arg(long)]
        account: Option<String>,
    },
    /// Mark a domain verified once its record has been seen.
    Confirm {
        /// The domain.
        domain: String,
        /// The account (default: the profile's).
        #[arg(long)]
        account: Option<String>,
    },
    /// List the account's domains.
    List {
        /// The account (default: the profile's).
        #[arg(long)]
        account: Option<String>,
    },
    /// Remove a domain; its record may then be deleted.
    Rm {
        /// The domain.
        domain: String,
        /// The account (default: the profile's).
        #[arg(long)]
        account: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConnectorsCommand {
    /// List the catalogue.
    List {
        /// Only connectors in this category.
        #[arg(long, value_enum)]
        category: Option<ConnectorCategory>,
    },
    /// Show one connector: sign-in modes, actions and hosts.
    ///
    /// One connector: its sign-in modes and their fields, settings, actions and the
    /// hosts it may call.
    Show {
        /// The connector's id, such as pagerduty.
        id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ConnectorCategory {
    Ai,
    Incident,
    Chat,
    Code,
    Observability,
    Enterprise,
    Generic,
}

impl ConnectorCategory {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ai => "ai",
            Self::Incident => "incident",
            Self::Chat => "chat",
            Self::Code => "code",
            Self::Observability => "observability",
            Self::Enterprise => "enterprise",
            Self::Generic => "generic",
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum ConnectionsCommand {
    /// List the account's connections.
    List {
        /// Only connections in this state.
        #[arg(long, value_enum)]
        status: Option<ConnectionStatus>,
        #[command(flatten)]
        account: AccountArg,
    },
    /// Show one connection: provider identity, health and grants.
    ///
    /// One connection: who it is at the provider, its health and grants. Never shows
    /// a credential.
    Show {
        /// The connection's name or id (con_...).
        connection: String,
        #[command(flatten)]
        account: AccountArg,
    },
    /// Connect an app with a key or by signing in.
    ///
    /// Connect an app. A key is asked for without echo, or read with --secret-file or
    /// --secret-stdin, never from an argument; signing in opens the provider in a
    /// browser and waits for it.
    Add(ConnectionsAdd),
    /// Run the connector's test request now and keep the outcome as the health.
    Test {
        /// The connection's name or id.
        connection: String,
        #[command(flatten)]
        account: AccountArg,
    },
    /// List every use of the connections, newest first.
    ///
    /// Every use of the account's connections, or of one, newest first: who, which
    /// action, the outcome; never the content.
    History {
        /// Only this connection's uses (name or id).
        connection: Option<String>,
        /// Only this action's ("test" for tests).
        #[arg(long)]
        action: Option<String>,
        /// Only this outcome.
        #[arg(long, value_enum)]
        status: Option<UseStatus>,
        /// Only this consumer's, as KIND:ID (product:reliability, key:ak_..., agent:NAME).
        #[arg(long)]
        consumer: Option<String>,
        /// At most this many.
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[command(flatten)]
        account: AccountArg,
    },
    /// Pause a connection: every call with it is refused until it is resumed.
    Pause {
        /// The connection's name or id.
        connection: String,
        #[command(flatten)]
        account: AccountArg,
    },
    /// Resume a paused connection.
    Resume {
        /// The connection's name or id.
        connection: String,
        #[command(flatten)]
        account: AccountArg,
    },
    /// Give a connection a new name (tools are named after it).
    Rename {
        /// The connection's name or id.
        connection: String,
        /// The new name: lowercase letters, digits and '-'.
        new_name: String,
        #[command(flatten)]
        account: AccountArg,
    },
    /// Delete a connection and destroy its credential.
    ///
    /// Delete a connection: its credential is destroyed and its grants stop working.
    /// Asks first unless --yes.
    Delete {
        /// The connection's name or id.
        connection: String,
        /// Do not ask.
        #[arg(long, short = 'y')]
        yes: bool,
        #[command(flatten)]
        account: AccountArg,
    },
    /// Give a connection a new credential.
    ///
    /// Give a connection a new credential: a key is asked for again, a signed-in
    /// connection opens the provider in a browser.
    Reconnect(ConnectionsReconnect),
    /// Grant a consumer actions of a connection.
    ///
    /// Grant a consumer named actions of a connection, until an expiry. A signed-in
    /// owner or admin only.
    Grant {
        /// The connection's name or id.
        connection: String,
        /// Who: product:NAME, key:ID, agent:NAME or avatar:ID.
        #[arg(long = "to", value_name = "KIND:ID")]
        to: String,
        /// The actions, separated with commas or repeated.
        #[arg(long, required = true, value_delimiter = ',')]
        actions: Vec<String>,
        /// How long it lasts: days such as 90d, or an RFC 3339 time (default: the
        /// platform's, 90 days).
        #[arg(long)]
        expires: Option<String>,
        #[command(flatten)]
        account: AccountArg,
    },
    /// The grants on a connection.
    Grants {
        /// The connection's name or id.
        connection: String,
        #[command(flatten)]
        account: AccountArg,
    },
    /// Revoke a grant; the next call it covered is refused.
    RevokeGrant {
        /// The connection's name or id.
        connection: String,
        /// The grant's id (gnt_...).
        grant: String,
        #[command(flatten)]
        account: AccountArg,
    },
}

/// `--account`, for commands that act on an account's objects.
#[derive(Debug, Clone, Args)]
pub struct AccountArg {
    /// The account (default: the profile's).
    #[arg(long)]
    pub account: Option<String>,
}

/// Where secret fields come from: never an argument.
#[derive(Debug, Clone, Args)]
pub struct SecretInput {
    /// Read a secret field from a file, as FIELD=PATH; repeat it for several fields.
    #[arg(long = "secret-file", value_name = "FIELD=PATH")]
    pub files: Vec<String>,
    /// Read one secret field from standard input.
    #[arg(long = "secret-stdin", value_name = "FIELD")]
    pub stdin: Option<String>,
}

#[derive(Debug, Args)]
pub struct ConnectionsAdd {
    /// The connector's id, such as incident-io (`iohr connectors list`).
    pub connector: String,
    /// The sign-in mode (default: the connector's first; `iohr connectors show` lists
    /// them).
    #[arg(long)]
    pub mode: Option<String>,
    /// The connection's name (default: the connector's id).
    #[arg(long)]
    pub name: Option<String>,
    /// A setting, as KEY=VALUE; repeat it. Never a secret.
    #[arg(long = "config", value_name = "KEY=VALUE")]
    pub config: Vec<String>,
    /// For a mode that signs in: a scope to ask for besides the defaults; repeat it or
    /// separate with commas.
    #[arg(long = "scope", value_delimiter = ',')]
    pub scopes: Vec<String>,
    #[command(flatten)]
    pub secrets: SecretInput,
    #[command(flatten)]
    pub account: AccountArg,
}

#[derive(Debug, Args)]
pub struct ConnectionsReconnect {
    /// The connection's name or id.
    pub connection: String,
    /// Switch to another of the connector's sign-in modes.
    #[arg(long)]
    pub mode: Option<String>,
    #[command(flatten)]
    pub secrets: SecretInput,
    #[command(flatten)]
    pub account: AccountArg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ConnectionStatus {
    Active,
    Paused,
    NeedsReauth,
    Error,
}

impl ConnectionStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::NeedsReauth => "needs_reauth",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum UseStatus {
    Ok,
    Failed,
    Refused,
}

impl UseStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::Refused => "refused",
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum ExtCommand {
    /// List every package you have, in one table.
    ///
    /// Every package you have, in one table: NAME, KIND, SOURCE (account, shared, local,
    /// installed), VISIBILITY, VERSION and INSTALLED. Published ones, private too, come
    /// from the API; local ones are the folders under ~/.inorbit/ext-dev, read from disk
    /// and never sent anywhere. An installed extension is matched to a package by name
    /// alone, since the lock keeps no publisher. With `--json`, an array of the same fields.
    Packages,
    /// Search the extensions catalogue.
    ///
    /// Search the extensions catalogue: name, kind, publisher, latest
    /// version and visibility. Reads the API only, never the registry.
    Search {
        /// Words to look for in the name, publisher and description.
        query: Option<String>,
        /// Only this kind: program, agent-plugin, web, connector or assistant-plugin.
        #[arg(long)]
        kind: Option<String>,
        /// Every match: follow the pages to the end.
        #[arg(long, conflicts_with = "page_size")]
        all: bool,
        /// At most this many (the catalogue's default page is 50).
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=200))]
        page_size: Option<u32>,
    },
    /// Show one extension in the catalogue, or list them all.
    ///
    /// One extension in the catalogue: what it does, its publisher, signer, scopes,
    /// privileges in plain words, evidence, versions and the line that installs it.
    /// With no name, the catalogue itself, the list `ext search` prints (a listing not
    /// released yet reads "coming").
    Show {
        /// PUBLISHER/NAME, such as inorbit/agent (default: list the catalogue).
        extension: Option<String>,
    },
    /// Verify and install an extension.
    ///
    /// Fetch an extension, verify its signature, provenance and digest, and install it.
    /// One that declares privileges for its system service asks first unless --yes.
    ///
    /// PUBLISHER/NAME resolves the version through the catalogue first; the artifact
    /// must then be the one the catalogue lists, signed by the signer it names, as well
    /// as pass every check the trust root makes.
    Install {
        /// NAME, NAME@VERSION or NAME@sha256:DIGEST (default: the newest release) from
        /// the registry, or PUBLISHER/NAME[@VERSION|@sha256:DIGEST] through the catalogue.
        extension: String,
        /// Also pin it in this lock file, for `iohr ext sync` elsewhere.
        #[arg(long, value_name = "FILE")]
        lock: Option<PathBuf>,
        /// Confirm the privileges it declares without asking (needed without a terminal),
        /// and set up its system service if it has one.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Don't set up its system service now (`iohr ext service NAME` does it later).
        #[arg(long)]
        no_service: bool,
        /// A network interface its system service attaches to; repeatable (default: asked
        /// at a terminal, from the ones the program detects; otherwise the default route's).
        #[arg(long = "interface", value_name = "NAME")]
        interfaces: Vec<String>,
        /// Every interface the program's --all picks (physical NICs, bonds, VLANs that are
        /// up), chosen again at each start of the service.
        #[arg(long, conflicts_with = "interfaces")]
        all_interfaces: bool,
    },
    /// Set up an extension's system service.
    ///
    /// Set up the system service of an installed extension that has one (Linux, with
    /// sudo): runs `sudo <program> service install --agent-user <you>`.
    Service {
        /// The extension, such as capture.
        name: String,
        /// A network interface it attaches to; repeatable (default: asked at a terminal).
        #[arg(long = "interface", value_name = "NAME")]
        interfaces: Vec<String>,
        /// Every interface the program's --all picks, chosen again at each start.
        #[arg(long, conflicts_with = "interfaces")]
        all_interfaces: bool,
    },
    /// The installed extensions.
    List,
    /// Install the newest release of an extension.
    ///
    /// Install the newest release of one extension, or of every installed one. A release
    /// that declares new privileges asks first unless --yes.
    Upgrade {
        /// The extension (default: all).
        name: Option<String>,
        /// Also pin the result in this lock file.
        #[arg(long, value_name = "FILE")]
        lock: Option<PathBuf>,
        /// Confirm new privileges without asking (needed without a terminal).
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Remove an extension from this machine.
    Remove {
        /// The extension.
        name: String,
        /// Also remove it from this lock file.
        #[arg(long, value_name = "FILE")]
        lock: Option<PathBuf>,
    },
    /// Check installed extensions, or one artifact, offline.
    ///
    /// Check installed extensions again, offline: the program's hash, the signature
    /// and provenance kept at install, and the signer the lock pins.
    ///
    /// Or check one artifact that is not installed: give a file, or a
    /// `sha256:DIGEST`, with the Sigstore bundle(s) for it in `--bundle` (what `cosign
    /// sign-blob --bundle` writes, the same file the cluster's init container checks) and,
    /// for a publisher's own key, `--publisher-key`. It runs the same trust check as an
    /// install, offline and with no sign-in, prints who signed, and exits 1 when the
    /// artifact is not trusted. With `--json`, one object: ok, digest, signer, provenance,
    /// error.
    Verify {
        /// The installed extension (default: all); or, for an artifact, a file path or a
        /// `sha256:DIGEST` (a path has a `/` or a `.`; an extension name has neither).
        #[arg(value_name = "NAME|ARTIFACT|DIGEST")]
        name: Option<String>,
        /// A Sigstore bundle attached to the artifact; repeatable. Artifact checks only.
        #[arg(long = "bundle", value_name = "FILE")]
        bundles: Vec<PathBuf>,
        /// A PEM public key the publisher signs with; repeatable (default: InOrbit's
        /// release workflow only, never the keys in the config). Artifact checks only.
        #[arg(long = "publisher-key", value_name = "PEM")]
        publisher_keys: Vec<PathBuf>,
    },
    /// Install exactly what a lock file pins, by digest and signer.
    Sync {
        /// The lock file.
        #[arg(long, default_value = "iohr-ext.lock", value_name = "FILE")]
        lock: PathBuf,
    },
    /// Start a new extension in a folder.
    ///
    /// Start a new extension: a folder with its manifest (every field, commented), a
    /// program that asks the token channel for a token and writes a heartbeat, a
    /// self-test, a README and a release workflow that signs it.
    Init {
        /// PUBLISHER/NAME, such as inorbit/hello. A name holding "tbd" is refused.
        extension: String,
        /// The kind; this release scaffolds programs.
        #[arg(long, value_enum, default_value_t = InitKind::Program)]
        kind: InitKind,
        /// The language of the program.
        #[arg(long, value_enum)]
        lang: crate::ext::manifest::Lang,
        /// The folder to write (default: ./NAME); it must not exist or be empty.
        #[arg(long, value_name = "DIR")]
        dir: Option<PathBuf>,
    },
    /// Run an extension from a folder.
    ///
    /// Run the extension in a folder: its manifest linted, the token channel for exactly
    /// its scopes (a refused request shown in red), its secrets, STATE and CACHE under
    /// ~/.inorbit/ext-dev/NAME/, and its output with the time. Never touches installed
    /// extensions or their lock.
    Dev {
        /// The extension's folder (default: this one).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Run its tests instead, with the same channel and environment; fails when they
        /// write outside STATE and CACHE.
        #[arg(long)]
        test: bool,
    },
    /// Check an extension's extension.toml.
    ///
    /// Check the extension.toml in a folder: unknown keys, values that look like
    /// secrets, data outside the non-sensitive kinds, unsupported targets, a
    /// "tbd" name, and every rule of the artifact. Exits 1 on any.
    Lint {
        /// The extension's folder (default: this one).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Print what it may do in plain words, as an installer shows it.
        #[arg(long)]
        preview: bool,
    },
}

#[derive(Debug, Args)]
pub struct AskArgs {
    /// The question or the job, in words (quotes are optional).
    #[arg(required_unless_present = "menu", conflicts_with = "menu", num_args = 1..)]
    pub question: Vec<String>,
    #[command(flatten)]
    pub headless: HeadlessArgs,
    /// Open the menu under a `?` card, for the shell hook: pick Run, Copy, Follow up, Explain,
    /// Open or Undo with a letter or the arrows. It asks no model, runs nothing, and prints the one
    /// line to put on the prompt (or nothing) on stdout.
    #[arg(long, conflicts_with_all = ["interactive", "headless", "card", "hook"])]
    pub menu: bool,
    /// With `--menu`: the command the answer suggested.
    #[arg(long, value_name = "COMMAND", requires = "menu")]
    pub command: Option<String>,
    /// With `--menu`: the file (relative to `--cwd`) or `https://` link the answer named.
    #[arg(long, value_name = "LINK", requires = "menu")]
    pub link: Option<String>,
    /// With `--menu`: the permission rules this answer's run saved, separated by the unit
    /// separator (U+001F). Undo takes back the ones the project file still holds.
    #[arg(
        long,
        value_name = "RULES",
        requires = "menu",
        value_delimiter = '\u{1f}'
    )]
    pub undo: Vec<String>,
    /// With `--menu`: the folder Io needs your OK to read (Allow keeps the yes for it).
    #[arg(long, value_name = "FOLDER", requires = "menu")]
    pub access: Option<std::path::PathBuf>,
    /// With `--menu`: the next steps the answer offered as `kind:value` (`read:api`,
    /// `search:TODO`, `run:cargo test`), separated by the unit separator (U+001F).
    #[arg(
        long,
        value_name = "STEPS",
        requires = "menu",
        value_delimiter = '\u{1f}'
    )]
    pub step: Vec<String>,
    /// With `--menu`: the question that was asked, so Allow and Attach output can put it back on
    /// the prompt to ask again.
    #[arg(long, value_name = "QUESTION", requires = "menu")]
    pub asked: Option<String>,
    /// With `--menu`: the last command, when it failed. With `--failed-exit` and `--asked`
    /// (a failure question) the menu offers to re-run it with its output captured, or to
    /// paste the output.
    #[arg(long, value_name = "COMMAND", requires = "menu")]
    pub failed_command: Option<String>,
    /// With `--menu`: the exit status of `--failed-command`.
    #[arg(long, value_name = "N", allow_hyphen_values = true, requires = "menu")]
    pub failed_exit: Option<i32>,
    /// With `--menu`: the file (private, made by the hook) the attached output is written to.
    #[arg(long, value_name = "FILE", requires = "menu")]
    pub output_to: Option<std::path::PathBuf>,
    /// Where the answer comes from: `local`, your own agent (default; `[ask] via`).
    #[arg(long, value_name = "WHERE")]
    pub via: Option<String>,
    /// The agent to start: `claude` (default; `[ask] agent`).
    #[arg(long, value_name = "AGENT")]
    pub agent: Option<String>,
    /// The model that answers: a Claude alias such as `haiku` or a full model name. Default
    /// `[ask] model`, else `haiku`, the small fast tier. With `--agent codex` it is passed on
    /// only when you name one.
    #[arg(long, value_name = "MODEL", env = "IOHR_ASK_MODEL")]
    pub model: Option<String>,
    /// Work in a Role: its text joins the prompt and its model is used when you name none
    /// (project `atlas/roles/`, then your account, then the built-in catalogue).
    #[arg(long, value_name = "NAME")]
    pub role: Option<String>,
    /// With `--role`: the job whose brief the Role compiles (its core, then matching recall).
    #[arg(long, value_name = "ID", requires = "role")]
    pub job: Option<String>,
    /// How Io works: `manual` asks before each command (default at a terminal), `plan` reads
    /// and proposes a plan and changes nothing (Claude's plan mode; Codex's read-only
    /// sandbox), `auto` runs commands no rule decides without asking (a deny rule and the
    /// secret list still win). In the shell hook shift+tab on a `?` line switches between
    /// them. Beats `--permissions`. `[ask] mode`, `IOHR_ASK_MODE`.
    #[arg(long, value_name = "MODE", env = "IOHR_ASK_MODE")]
    pub mode: Option<String>,
    /// Let the agent edit files in this folder (Claude `--permission-mode acceptEdits`, Codex
    /// `--sandbox workspace-write`). Without it nothing is changed: print mode refuses edits
    /// rather than ask. Shell commands are never allowed by iohr. It is the plain run's `--mode
    /// auto` (`--mode plan` wins over it). `[ask] allow_edits`.
    #[arg(long)]
    pub allow_edits: bool,
    /// What happens when Io wants to run a command or open a file no rule decides: `manual`
    /// (or `prompt`) asks you at the terminal (default at a terminal), `plan` and `auto` are
    /// the modes of `--mode`, `rules` only follows your rules and says no to the rest (default
    /// without a terminal), `deny` offers no commands at all. `--mode` wins over it. Rules:
    /// `iohr permissions`. `[ask] permissions`.
    #[arg(long, value_name = "HOW", env = "IOHR_ASK_PERMISSIONS")]
    pub permissions: Option<String>,
    /// How the thinking is drawn at a terminal: `show` (streamed, dim, in a quiet block;
    /// default) or `collapse` (one line). `[ask] thinking`.
    #[arg(long, value_name = "HOW")]
    pub thinking: Option<String>,
    /// Print the exact command line the agent would run and start nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// The atlas kinds a document can be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AtlasKind {
    Prd,
    Rfc,
    Adr,
    Study,
}

#[derive(Debug, Subcommand)]
pub enum AtlasCommand {
    /// Add the atlas folder to a project that has none.
    ///
    /// Writes the folders and starters `iohr project init` writes, for a project made without
    /// them. Only what is missing: a file or folder you already have is left as it is.
    Init {
        /// The project folder. Default: the folder you are in.
        #[arg(long, value_name = "DIR")]
        dir: Option<PathBuf>,
    },
    /// Add a new PRD, RFC, ADR or study with the next free number.
    New {
        /// What kind of document.
        #[arg(value_enum)]
        kind: AtlasKind,
        /// Its title, a few words: it becomes the file name.
        title: String,
        /// The project folder. Default: the nearest folder up from here that has `atlas/`.
        #[arg(long, value_name = "DIR")]
        dir: Option<PathBuf>,
    },
    /// Send the atlas documents to the project's Decisions space.
    Sync(SyncArgs),
    /// The built-in roles of this project.
    #[command(subcommand, alias = "personalities")]
    Roles(RolesCommand),
}

#[derive(Debug, Subcommand)]
pub enum RolesCommand {
    /// Show the project's roles and whether InOrbit has them.
    ///
    /// The built-in roles in `atlas/roles/`: their size and sources, and, when
    /// you are signed in, the version InOrbit holds.
    List {
        /// The project folder. Default: the nearest folder up from here that has `atlas/`.
        #[arg(long, value_name = "DIR")]
        dir: Option<PathBuf>,
    },
    /// Import the project's roles into your InOrbit account.
    ///
    /// The same import `iohr sync` does. A built-in role already in your account that nobody
    /// changed is upgraded to the catalogue's newer version; one you changed, or that is not
    /// a built-in, is left alone (`--upgrade` takes the catalogue's version over a changed one).
    Import {
        /// Say what would be sent and send nothing.
        #[arg(long)]
        dry_run: bool,
        /// Also replace a copy you changed with the catalogue's newer version (InOrbit keeps
        /// the old version, so it can be reverted).
        #[arg(long)]
        upgrade: bool,
        /// The project folder. Default: the nearest folder up from here that has `atlas/`.
        #[arg(long, value_name = "DIR")]
        dir: Option<PathBuf>,
    },
}

/// Which rule file: the project's or yours. Neither means the default for the command.
#[derive(Debug, Clone, Copy, Args)]
pub struct PermissionScope {
    /// The project's `.inorbit/permissions.toml` (committed with the code).
    #[arg(long, conflicts_with = "user")]
    pub project: bool,
    /// Your own file in the config folder (every project).
    #[arg(long)]
    pub user: bool,
}

#[derive(Debug, Subcommand)]
pub enum PermissionsCommand {
    /// Show the rules, and where each one is kept.
    List {
        #[command(flatten)]
        scope: PermissionScope,
    },
    /// Allow a command or file, like `Bash(ls:*)` or `Read(./src/**)`.
    ///
    /// Default: the project file. Refused for anything that could open keys, `.env` files or
    /// credentials, and for a rule that would allow every command.
    Allow {
        /// The rule, in Claude Code's grammar.
        rule: String,
        #[command(flatten)]
        scope: PermissionScope,
    },
    /// Never allow a command or file; this beats any allow rule.
    Deny {
        /// The rule, in Claude Code's grammar.
        rule: String,
        #[command(flatten)]
        scope: PermissionScope,
    },
    /// Forget a rule, from the allow or deny list, in both files unless one is named.
    Remove {
        /// The rule, exactly as `list` shows it.
        rule: String,
        #[command(flatten)]
        scope: PermissionScope,
    },
    /// Show what the rules say about one call, without running it.
    ///
    /// `iohr permissions check Bash 'git status'`, `... check Read ./src/main.rs`,
    /// `... check WebFetch https://example.com`. Says allow, deny or ask, and which rule.
    Check {
        /// The tool: Bash, Read, Edit, Write, Grep, Glob or `WebFetch`.
        tool: String,
        /// The command, the file or the address.
        input: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum MemoryCommand {
    /// Keep a fact or a preference.
    Remember {
        /// What to keep, one short sentence. No secrets or personal data.
        text: String,
        /// Why it holds, so it can be checked later. `iohr memory check` flags a memory without.
        #[arg(long, value_name = "WHY")]
        why: Option<String>,
        /// A file or folder it is about (relative to the project), or `cmd:<command>`. Repeat it.
        #[arg(long = "ref", value_name = "PATH", action = clap::ArgAction::Append)]
        refs: Vec<String>,
        /// Keep it as a preference (how you want to be answered), not a fact.
        #[arg(long, conflicts_with = "kind")]
        preference: bool,
        /// The kind: fact, preference or episode. Default: fact.
        #[arg(long, value_name = "KIND", value_parser = ["fact", "preference", "episode"])]
        kind: Option<String>,
        /// Never call it stale, however long since it was confirmed.
        #[arg(long)]
        pin: bool,
        /// Where to keep it: project, user or account. Default: `[memory] scope`, else project.
        #[arg(long, value_name = "SCOPE", value_parser = ["project", "user", "account"])]
        scope: Option<String>,
    },
    /// Find what is known about a topic.
    Recall {
        /// The topic, in words.
        query: String,
        /// At most this many entries.
        #[arg(long, default_value_t = 10, value_name = "N")]
        limit: u32,
        /// Where to look: project, user or account. Default: `[memory] scope`, else project.
        #[arg(long, value_name = "SCOPE", value_parser = ["project", "user", "account"])]
        scope: Option<String>,
    },
    /// List what is kept.
    List {
        /// At most this many entries.
        #[arg(long, default_value_t = 50, value_name = "N")]
        limit: usize,
        /// Where to look: project, user or account. Default: `[memory] scope`, else project.
        #[arg(long, value_name = "SCOPE", value_parser = ["project", "user", "account"])]
        scope: Option<String>,
    },
    /// Forget one entry by id.
    Forget {
        /// The entry's id, from `iohr memory list`.
        id: String,
        /// Where it is kept: project, user or account. Default: `[memory] scope`, else project.
        #[arg(long, value_name = "SCOPE", value_parser = ["project", "user", "account"])]
        scope: Option<String>,
    },
    /// Find memories that are wrong, stale, repeated or missing.
    ///
    /// Reads the memories on this machine, with no network: refs that no longer exist, no
    /// reason given, not confirmed for a long time, no `checked:` date or an old one, the same thing twice, and what a
    /// role expects a project to remember (language, how to build and test, owners).
    Check {
        /// Where to look: project or user. Default: `[memory] scope`, else project.
        #[arg(long, value_name = "SCOPE", value_parser = ["project", "user"])]
        scope: Option<String>,
        /// Exit non-zero when anything is found.
        #[arg(long)]
        strict: bool,
        /// Days without a confirmation after which a memory is stale.
        #[arg(long, default_value_t = 180, value_name = "DAYS")]
        stale_days: u32,
        /// Days after which a memory's `checked:` date is too old. Default: `[memory]
        /// check_days`, else 180.
        #[arg(long, value_name = "DAYS")]
        checked_days: Option<u32>,
    },
    /// Compare memories with the code, and with the service when sync is on.
    ///
    /// Asks git whether the files a memory is about changed after it was last confirmed. With
    /// `[memory] sync = true` or `--remote`, also compares the files with the memory service:
    /// only here, only there, changed on both sides. It only reads; nothing is changed.
    Drift {
        /// Where to look: project or user. Default: `[memory] scope`, else project.
        #[arg(long, value_name = "SCOPE", value_parser = ["project", "user"])]
        scope: Option<String>,
        /// Compare with the memory service too, even when sync is off.
        #[arg(long)]
        remote: bool,
        /// Exit non-zero when anything drifted.
        #[arg(long)]
        strict: bool,
    },
    /// Sync memories with your InOrbit account.
    ///
    /// Off unless `[memory] sync = true` or you run this. Sends new and changed memories, links
    /// ones the service already has, and stops on a memory changed on both sides: nothing is
    /// overwritten. The service keeps no versions, so a change is detected here by hash and
    /// replaced as create-then-delete.
    Sync {
        /// What to sync: project or user. Default: `[memory] scope`, else project.
        #[arg(long, value_name = "SCOPE", value_parser = ["project", "user"])]
        scope: Option<String>,
        /// Say what would be sent and send nothing.
        #[arg(long)]
        dry_run: bool,
        /// Also write the service's entries that have no file here as new files.
        #[arg(long)]
        pull: bool,
    },
}

#[derive(Debug, Clone, Args)]
pub struct SessionArgs {
    /// The project folder (default: here).
    #[arg(value_name = "DIR")]
    pub dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct StopArgs {
    /// The project folder (default: here).
    #[arg(value_name = "DIR")]
    pub dir: Option<PathBuf>,
    /// Seconds to wait for the host to end (0 to 600).
    #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u64).range(0..=600))]
    pub wait: u64,
}

#[derive(Debug, Clone, Args)]
pub struct SyncArgs {
    /// Say what would be created or saved and send nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// The Decisions space id (`lab_...`). Default: the one in `.inorbit/space`.
    #[arg(long, value_name = "ID")]
    pub space: Option<String>,
    /// The project folder. Default: the nearest folder up from here that has `atlas/`.
    #[arg(long, value_name = "DIR")]
    pub dir: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum ShellCommand {
    /// Print the hook for a shell, to be evaluated by it: `? question` (and `?? follow-up`)
    /// at the prompt asks `iohr ask` with the folder and the last command, and the shell
    /// never runs that line. Only zsh is built; bash and fish say so and exit 1.
    Hook {
        /// The shell: `zsh` (`bash` and `fish` are not built yet).
        #[arg(value_enum)]
        shell: HookShell,
    },
}

/// A shell `iohr shell hook` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum HookShell {
    Zsh,
    Bash,
    Fish,
}

/// What makes `iohr ask` answer once and print (one capped, read-only run) instead of
/// streaming the agent's run: the way a shell prompt, a script or the InOrbit app asks
/// (PRD 0187). Any of `--headless`, `--card` or a context flag selects it.
#[derive(Debug, Args, Default)]
#[allow(clippy::struct_excessive_bools)] // independent command-line switches, not a state
pub struct HeadlessArgs {
    /// Answer once and print (a small model, a turn cap, no tools), even at a terminal.
    #[arg(long, conflicts_with = "interactive")]
    pub headless: bool,
    /// Stream the agent's run (the default), even with a flag that asks for one printed answer.
    #[arg(long)]
    pub interactive: bool,
    /// Draw the answer for a person on stderr; stdout still gets the plain answer.
    #[arg(long, conflicts_with = "interactive")]
    pub card: bool,
    /// For the shell hook: draw the card on stderr, with the action row, and print on stdout
    /// the answer, the suggested command and the link, separated by the 0x1e character.
    #[arg(long, conflicts_with = "interactive")]
    pub hook: bool,
    /// The folder the question is about (default: this one). Sent to the agent as text.
    #[arg(long, value_name = "DIR", conflicts_with = "interactive")]
    pub cwd: Option<String>,
    /// The last command the person ran, as context (cut to 1 KiB).
    #[arg(long, value_name = "COMMAND", conflicts_with = "interactive")]
    pub last_command: Option<String>,
    /// That command's exit status.
    #[arg(
        long,
        value_name = "N",
        allow_hyphen_values = true,
        conflicts_with = "interactive"
    )]
    pub exit_code: Option<i32>,
    /// A file with that command's output; only its last 200 lines or 8 KiB are used, and
    /// secrets in it are redacted first. Without it the card says the output was not sent.
    #[arg(long, value_name = "FILE", conflicts_with = "interactive")]
    pub output_file: Option<std::path::PathBuf>,
    /// Where `--output-file` came from, for the card's footer and `--json`: `attached` (a
    /// file you gave, the default), `captured` (a re-run you allowed) or `pasted`.
    #[arg(
        long,
        value_name = "HOW",
        value_parser = ["attached", "captured", "pasted"],
        requires = "output_file",
        conflicts_with = "interactive"
    )]
    pub output_kind: Option<String>,
    /// An earlier question this one follows up (with `--prior-answer`).
    #[arg(
        long,
        value_name = "TEXT",
        requires = "prior_answer",
        conflicts_with = "interactive"
    )]
    pub prior_question: Option<String>,
    /// The earlier question's answer.
    #[arg(
        long,
        value_name = "TEXT",
        requires = "prior_question",
        conflicts_with = "interactive"
    )]
    pub prior_answer: Option<String>,
    /// A folder you allow Io to read for this question, besides the one it is asked in: the
    /// answer to the card's "needs your OK to read" note, for a run with no terminal to ask on
    /// (a script, `--json`, the app). Repeat it for more folders; `--add-dir` is the same flag,
    /// as in Claude Code. `IOHR_ASK_READ` (a path list) and `[ask] read` add to these. Secrets
    /// stay blocked.
    #[arg(
        long,
        visible_alias = "add-dir",
        value_name = "FOLDER",
        conflicts_with = "interactive"
    )]
    pub read: Vec<std::path::PathBuf>,
}

impl HeadlessArgs {
    /// True when a context flag was given: only a headless run reads them.
    #[must_use]
    pub fn has_context(&self) -> bool {
        self.cwd.is_some()
            || self.last_command.is_some()
            || self.exit_code.is_some()
            || self.output_file.is_some()
            || self.prior_question.is_some()
            || !self.read.is_empty()
    }
}

/// Which Planner plan `iohr project init` uses (RFC 0211).
#[derive(Debug, Args)]
pub struct InitPlanArgs {
    /// Use this existing Planner plan for the project instead of making one: its id is
    /// written under `[plan]` in `coordination.toml` (checked when you are signed in).
    /// Without it, a signed-in init makes the plan named after the project, or finds the
    /// one it made before; signed out, the id stays empty and init says how to set it.
    #[arg(long = "plan-id", value_name = "PLAN")]
    pub plan_id: Option<String>,
    /// Leave `[plan] id` in `coordination.toml` empty: no plan is made or looked up.
    #[arg(long = "no-plan", conflicts_with = "plan_id")]
    pub no_plan: bool,
}

/// The flags of `project init`: one struct, boxed in [`ProjectCommand::Init`] because the
/// variant is far larger than its siblings (clippy `large_enum_variant`, seen on Windows where
/// `PathBuf` is wider).
#[derive(Debug, Args)]
pub struct InitArgs {
    /// The project's name: lowercase letters, digits and dashes, like demo or my-shop.
    /// At a terminal it is asked for when missing (the folder's name is the default);
    /// anywhere else it is required.
    pub name: Option<String>,
    /// The template: an id (`iohr project template list` shows them), a folder
    /// (`./my-template`), or a git repository (`gh:owner/repo[//subdir][@ref]`,
    /// `git:URL[//subdir][@ref]`). Default: `[project] template` in iohr's config.toml,
    /// else coordinated. A template that is coming says so and refuses. At a terminal, leaving
    /// it out offers the templates as a menu.
    #[arg(long)]
    pub template: Option<String>,
    /// Answer a template question: `--set key=value`, repeatable.
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub set: Vec<String>,
    /// Answers from a .json or .toml file (a table of key to value); `--set` wins.
    #[arg(long, value_name = "FILE")]
    pub answers: Option<PathBuf>,
    /// Describe the project and your own agent (claude or codex, on your PATH) builds it
    /// in the new folder, from the template's agent prompt. It runs in the background of
    /// this command, in print mode (no session opens): it may edit files, read, search and fetch from the web,
    /// and run only the language's own build and test commands (cargo, go, python, node), a few
    /// read-only ones (git status, log and diff, ls) and this program's `atlas new`,
    /// `atlas sync --dry-run` and `memory` commands, never any other shell command (no curl);
    /// iohr draws its progress. The smoke test runs after it. Its output is
    /// labelled AI. `--json` passes the agent's events through as JSON lines.
    #[arg(long, value_name = "TEXT")]
    pub describe: Option<String>,
    /// The most turns the agent may take (Claude only; Codex has no turn limit). Default
    /// `[project] agent_turns`, else 150, init's own: `[ask]` in config.toml has no bound.
    #[arg(long, value_name = "N")]
    pub agent_turns: Option<u32>,
    /// The most seconds the agent may run before iohr stops it. Default `[project]
    /// agent_timeout`, else 1800, init's own. When it stops at a cap, iohr says so,
    /// writes `.inorbit/handoff.md` and prints how to continue.
    #[arg(long, value_name = "SECONDS")]
    pub agent_timeout: Option<u64>,
    /// The model the agent builds with: a Claude alias such as `sonnet` or a full model
    /// name. Default `[project] agent_model`, else `sonnet` (a whole site needs more than
    /// the small fast tier `iohr ask` uses). With Codex it is passed on only when named.
    #[arg(long, value_name = "MODEL", env = "IOHR_INIT_AGENT_MODEL")]
    pub agent_model: Option<String>,
    /// Build in a Role: its text joins the agent's prompt and its model is used when you
    /// name none (the new project's `atlas/roles/`, then your account, then the built-in
    /// catalogue).
    #[arg(long, value_name = "NAME")]
    pub role: Option<String>,
    /// Run the whole flow (menu, questions, defaults, `when` skips) and change nothing:
    /// no files, no git, no registration, no smoke test, no sign-in, no network, and the
    /// agent is not started (its command and prompt are printed). Every step shows as
    /// `would: ...`; with `--json` the plan is data. `--plan` is the same.
    #[arg(long, visible_alias = "plan")]
    pub dry_run: bool,
    #[command(flatten)]
    pub plan: InitPlanArgs,
    /// Answer the questions with their defaults: no menu, no confirmation.
    #[arg(short = 'y', long)]
    pub yes: bool,
    /// The folder to write, relative to where you are. Default: ./NAME; at a terminal a
    /// menu proposes the folder you are in, `[project] root` in config.toml, a sibling
    /// when you are inside a git repository, or a path of your own. It must not exist
    /// or be empty, or be the folder this project was made in: run again there, init
    /// leaves every file that is already there alone and commits nothing.
    #[arg(long, value_name = "DIR")]
    pub dir: Option<PathBuf>,
    /// The language, for a template that offers several (the same as `--set
    /// language=ID`); `iohr project template show ID` lists them. At a terminal,
    /// leaving it out offers a menu that marks the toolchains you have.
    #[arg(long, value_name = "ID")]
    pub lang: Option<String>,
    /// The publisher half of the extension id (default: your account's slug, else
    /// "local"); the private listing takes it when that step lands.
    #[arg(long)]
    pub publisher: Option<String>,
    /// Skip the smoke test (it runs the project once with the language's own runtime).
    #[arg(long)]
    pub no_smoke: bool,
    /// Where the project lives in InOrbit: team, who reads it, or none.
    #[command(flatten)]
    pub space: InitSpaceArgs,
}

/// The flags of `project init` that say where the project lives in InOrbit.
#[derive(Debug, Args)]
pub struct InitSpaceArgs {
    /// The team the project lives under in InOrbit: an account's id or slug
    /// (`iohr accounts list` shows yours). Default: the account you are signed in on; at a
    /// terminal, with more than one account, init asks. The environment is the global
    /// `--env` (`production` by default, or `local`).
    #[arg(long, value_name = "ACCOUNT")]
    pub team: Option<String>,
    /// Who can see the project in that account: `team` (everyone in it; only you in your own
    /// account, the default) or `managers` (its owners and admins). At a terminal, init asks.
    #[arg(long, value_name = "WHO", value_parser = clap::builder::PossibleValuesParser::new(crate::project::space::Readers::NAMES))]
    pub visibility: Option<String>,
    /// Keep the project on this machine: do not create it in InOrbit or queue that.
    #[arg(long)]
    pub no_sync: bool,
}

/// The flags of `project sync`.
#[derive(Debug, Args)]
pub struct ProjectSyncArgs {
    /// The project folder. Default: the folder you are in.
    #[arg(long, value_name = "DIR")]
    pub dir: Option<PathBuf>,
    /// The team the project lives under: an account's id or slug. Default: what init
    /// queued, else the account you are signed in on.
    #[arg(long, value_name = "ACCOUNT")]
    pub team: Option<String>,
    /// Who can see the project in that account: `team` or `managers`. Default: what init
    /// queued, else `team`.
    #[arg(long, value_name = "WHO", value_parser = clap::builder::PossibleValuesParser::new(crate::project::space::Readers::NAMES))]
    pub visibility: Option<String>,
    /// Say what would be made or linked and change nothing.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// Create a project from a template.
    ///
    /// Set up a project of your own: sign-in checked, the folder scaffolded from a
    /// template with its manifest, a git repository with one commit, its own state under
    /// ~/.inorbit/projects/NAME/, a Keychain slot by name, a smoke test that runs it once,
    /// and a report. Idempotent: run again in the folder it made, it leaves every file that
    /// is already there as it is and says which, and a step that waits on an API says so
    /// instead of pretending. `iohr init` and `create` are the same command.
    #[command(visible_alias = "create")]
    Init(Box<InitArgs>),
    /// Create the project in InOrbit, or link to it, from its folder.
    ///
    /// The part of `init` that makes the project in InOrbit: a Decisions space of kind
    /// project under your team, for the environment `--env` names. Idempotent: it finds the
    /// space by the project's name and links to it, so a repeat, a retry after init could not
    /// reach InOrbit, and a teammate who cloned the project all end at the same space. The
    /// link is written to `.inorbit/space` (production, committed) or `.inorbit/space.local`.
    Sync(ProjectSyncArgs),
    /// The same as `template list`, kept so older scripts work.
    #[command(hide = true)]
    Templates,
    /// List, show, make and check project templates.
    ///
    /// Templates are folders with a template.toml: the built-in ones, and your own in
    /// ~/.inorbit/templates/. `list` shows them, `show` says what one asks and writes,
    /// `new` scaffolds one of yours, `lint` checks a folder, `path` prints where one lives
    /// and `schema` prints the JSON Schema of template.toml.
    Template {
        #[command(subcommand)]
        cmd: TemplateCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum TemplateCommand {
    /// List the templates `init` accepts.
    ///
    /// The templates `init` and `create` accept: name, live or coming, where each comes
    /// from, which one is the default (`[project] template` in config.toml, else coordinated) and
    /// what it is. `--json` prints the same as a list.
    List,
    /// Show what a template asks, writes and runs.
    ///
    /// Show one template: where it comes from, what it extends, every question (id, type,
    /// default and `when`), the files it renders and its agent step. The id may also be a
    /// folder or a git repository, as for `init --template`. `--json` prints all of it as
    /// one object.
    Show {
        /// The template: an id, a folder (`./my-template`) or `gh:owner/repo[//subdir][@ref]`.
        id: String,
    },
    /// Print the folder a template lives in.
    ///
    /// Print only the folder of a template of yours, so `cd $(iohr project template path
    /// demo)` works. A built-in template is part of iohr and has no folder: exit 1, with
    /// the command that makes an editable copy.
    Path {
        /// The template's id.
        id: String,
    },
    /// Scaffold a template of your own.
    ///
    /// Scaffold a template of your own in ~/.inorbit/templates/ID, a working one to edit.
    New {
        /// The template's id: lowercase letters, digits and dashes.
        id: String,
        /// Add to another template (an id): its files, questions and steps come first.
        #[arg(long, value_name = "ID")]
        extends: Option<String>,
    },
    /// Check a template folder.
    ///
    /// Check a template folder: its template.toml, its questions and `when` rules, every
    /// placeholder, `extends`, and the extension manifest it renders. Exits 1 on any finding.
    Lint {
        /// The template's folder (default: this one).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Print the JSON Schema of template.toml.
    ///
    /// Print the JSON Schema of template.toml, for editors and InOrbit Desktop.
    Schema,
}

/// What `iohr ext init` scaffolds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InitKind {
    /// A program that runs as its own process.
    Program,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Set a value: `ext.registry HOST/PATH`, or `ext.trusted_keys FILE.pem...`.
    Set {
        /// The key.
        key: ConfigKey,
        /// The value; for `ext.trusted_keys`, one or more PEM public-key files.
        #[arg(required = true, num_args = 1..)]
        values: Vec<String>,
    },
    /// Print a value.
    Get {
        /// The key.
        key: ConfigKey,
    },
    /// Return a value to its default.
    Unset {
        /// The key.
        key: ConfigKey,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ConfigKey {
    /// The registry and path prefix extensions come from.
    #[value(name = "ext.registry")]
    ExtRegistry,
    /// Public keys trusted to sign extensions besides InOrbit's release workflow.
    #[value(name = "ext.trusted_keys")]
    ExtTrustedKeys,
}

#[derive(Debug, Args)]
pub struct Login {
    #[command(flatten)]
    pub how: How,
    /// Keep the credential in an owner-only file instead of the OS credential store.
    #[arg(long)]
    pub insecure_storage: bool,
    /// A scope to ask for besides the defaults, such as the runner's
    /// `coordinator:runner,coordinator:read`; repeat it or separate with commas. The
    /// sign-in service refuses a scope the command line's client may not ask for.
    #[arg(long = "scope", value_delimiter = ',')]
    pub scopes: Vec<String>,
    /// The account this profile acts for, by id or slug, among the ones you belong to.
    /// It is kept when you sign in again. Without it: the account this profile was last
    /// pointed at, else the one you made your default, else your own.
    #[arg(long, value_name = "ID|SLUG", conflicts_with = "with_token")]
    pub account: Option<String>,
}

/// How to sign in; at most one. Without any, a browser when this machine can open
/// one, a device code otherwise.
#[derive(Debug, Args)]
#[group(multiple = false)]
pub struct How {
    /// Sign in in a browser on this machine.
    #[arg(long)]
    pub web: bool,
    /// Sign in with a code approved in any browser, on any device.
    #[arg(long)]
    pub device: bool,
    /// Read an API token from standard input instead of signing in.
    #[arg(long)]
    pub with_token: bool,
}

#[derive(Debug, Subcommand)]
pub enum ProfileCommand {
    /// List the profiles.
    List,
    /// Make a profile the default.
    Use {
        /// The profile.
        name: ProfileName,
    },
    /// Show one profile (default: the active one). Never shows a secret.
    Show {
        /// The profile.
        name: Option<ProfileName>,
    },
    /// Point a profile at one of its accounts.
    ///
    /// Point a signed-in profile at one of its accounts (a team's id or slug), the one
    /// `iohr sdk generate` cuts the document to.
    Account {
        /// The profile.
        name: ProfileName,
        /// The account's id or slug, among the ones the profile's person belongs to.
        account: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum AccountsCommand {
    /// List them.
    List,
}

#[derive(Debug, Subcommand)]
pub enum TokenCommand {
    /// Create an API token. It is printed once, alone on stdout.
    Create {
        /// A name that says where the token will be used.
        #[arg(long)]
        name: String,
        /// A scope the token holds; repeat it or separate with commas.
        #[arg(long = "scope", required = true, value_delimiter = ',')]
        scopes: Vec<String>,
        /// How long it lives, in days (7, 30, 90 or 365).
        #[arg(long, default_value_t = 30)]
        days: u32,
        /// The account (default: the profile's).
        #[arg(long)]
        account: Option<String>,
    },
    /// List API tokens.
    List {
        /// The account (default: the profile's).
        #[arg(long)]
        account: Option<String>,
        /// Only tokens in this state.
        #[arg(long, value_enum)]
        status: Option<TokenStatus>,
        /// Only tokens whose name or id starts with this.
        #[arg(long)]
        query: Option<String>,
        /// At most this many.
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Revoke an API token; calls with it are refused within seconds.
    Revoke {
        /// The token's id.
        id: String,
        /// The account (default: the profile's).
        #[arg(long)]
        account: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TokenStatus {
    Active,
    Expiring,
    Expired,
    Revoked,
}

impl TokenStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Expiring => "expiring",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
        }
    }
}

#[derive(Debug, Args)]
pub struct ApiCall {
    /// GET, POST, PUT, PATCH or DELETE.
    pub method: HttpMethod,
    /// A path on the API, such as /v1/me.
    pub path: String,
    /// A string field: a query value for GET and DELETE, a JSON body field otherwise.
    #[arg(short = 'f', long = "field", value_name = "KEY=VALUE")]
    pub fields: Vec<String>,
    /// A typed field: the value is read as JSON (numbers, true, false, null, arrays).
    #[arg(short = 'F', long = "typed-field", value_name = "KEY=JSON")]
    pub typed_fields: Vec<String>,
    /// The JSON body, from a file or `-` for standard input.
    #[arg(long, value_name = "FILE", conflicts_with_all = ["fields", "typed_fields"])]
    pub input: Option<PathBuf>,
    /// Print the status and the request id before the body.
    #[arg(short, long)]
    pub include: bool,
    /// GET every page of a list: follow `next_page_token` until it is empty and print
    /// one answer with every item.
    #[arg(long, visible_alias = "paginate")]
    pub all: bool,
    /// With --all, stop after this many pages; the answer then keeps the token to go on
    /// with (`-f page_token=...`).
    #[arg(long, value_name = "N", default_value_t = 100, requires = "all",
          value_parser = clap::value_parser!(u32).range(1..=10_000))]
    pub max_pages: u32,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "UPPER")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Debug, Subcommand)]
pub enum OpenapiCommand {
    /// Save the document this credential sees.
    Pull {
        /// The file to write, or `-` for standard output.
        #[arg(short, long, default_value = "openapi.json")]
        output: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
pub enum SdkCommand {
    /// Write an SDK cut to what your profiles may call.
    ///
    /// Write a surface cut to what your profiles may call into a directory, with its
    /// `iohr.lock` beside it.
    Generate(SdkGenerate),
    /// Fail when the cut has moved since it was generated.
    ///
    /// Fetch every profile's document again and fail when the cut has moved since the
    /// surface was generated (run it in CI with `IOHR_TOKEN_<PROFILE>` per profile).
    Check(SdkCheck),
    /// Write a snippet per operation and language.
    ///
    /// Write a short program per operation and language that calls it with the published
    /// runtime, as JSON keyed by operation id: what the API reference shows beside each
    /// operation. Every snippet builds against the runtime's public surface.
    Examples(SdkExamples),
    /// Print the configuration an SDK client would use here.
    ///
    /// Print the configuration an SDK client built with `load` would use here: every
    /// setting with where its value came from, the credential chain, the pipeline and
    /// what was ignored, as the SDKs' `describe()` gives it (`docs/config.md` section
    /// 2.6). Secrets are redacted. Reads the environment (`INORBIT_*`) and the config
    /// file; contacts no host. `--profile NAME` stands for `profile` in the client's
    /// code; `IOHR_PROFILE` is not read, as the SDKs do not read it.
    Config(SdkConfig),
    /// Add the published SDK to this project.
    ///
    /// Add the published SDK to the project here with the package manager it already
    /// uses: `cargo add`, `npm install` (or pnpm, yarn, bun, deno), `uv add` (or poetry,
    /// pdm, pip in the active virtualenv) or `go get`.
    ///
    /// The language is read from the nearest project file between this directory and the
    /// repository root when it is not given. The command is printed first and then run,
    /// as a program with its arguments (no shell, never sudo); iohr itself contacts no
    /// host, the package manager reaches its own registry. Exit code: the package
    /// manager's, or 2 when no project or manager can be chosen.
    #[command(disable_version_flag = true)]
    Add(SdkAdd),
}

#[derive(Debug, Args)]
pub struct SdkAdd {
    /// rust, typescript (ts, js), python (py) or go. Read from the project when left out.
    #[arg(value_enum)]
    pub lang: Option<AddLang>,
    /// Install this version, such as 0.2.1 (default: the newest release, recorded the way
    /// the package manager records it).
    #[arg(long, value_name = "VERSION")]
    pub version: Option<String>,
    /// Print the command and run nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// The languages `iohr sdk add` knows; C# and Java are refused until they are published.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum AddLang {
    Rust,
    #[value(alias = "ts", alias = "js")]
    Typescript,
    #[value(alias = "py")]
    Python,
    Go,
    Csharp,
    Java,
}

#[derive(Debug, Args)]
pub struct SdkConfig {
    /// Resolve for a generated profile type of this name (`Client<AcmeCi>`), which reads
    /// `INORBIT_<NAME>_*` and its own table, instead of the public client.
    #[arg(long = "for", value_name = "PROFILE")]
    pub profile_type: Option<ProfileName>,
}

#[derive(Debug, Args)]
pub struct SdkExamples {
    /// A language; repeat it for several. Every language when left out.
    #[arg(long, value_enum)]
    pub lang: Vec<Lang>,
    /// The document to write examples for, as NAME=FILE or FILE (the public document,
    /// `iohr openapi pull` or the platform's `openapi.public.json`).
    #[arg(long = "from", value_name = "[NAME=]FILE")]
    pub from: String,
    /// The file to write, or `-` for standard output.
    #[arg(long, value_name = "FILE", default_value = "-")]
    pub out: PathBuf,
}

#[derive(Debug, Args)]
pub struct SdkGenerate {
    /// The language.
    #[arg(long, value_enum)]
    pub lang: Lang,
    /// A profile whose credential's document to generate from; repeat it to put several
    /// accounts in one surface (`--profile` on its own is the global flag).
    #[arg(long = "for", value_name = "PROFILE")]
    pub profiles: Vec<ProfileName>,
    /// A document saved with `iohr openapi pull`, as NAME=FILE, for an offline run.
    #[arg(long = "from", value_name = "NAME=FILE")]
    pub from: Vec<String>,
    /// The directory to write the surface into; `iohr.lock` goes beside it.
    #[arg(long, value_name = "DIR")]
    pub out: PathBuf,
    /// Replace a non-empty output directory.
    #[arg(long)]
    pub force: bool,
    #[command(flatten)]
    pub options: iohr_codegen::Options,
}

#[derive(Debug, Args)]
pub struct SdkCheck {
    /// The lock file the surface was generated with.
    #[arg(long, default_value = "iohr.lock")]
    pub lock: PathBuf,
    /// Also render the surface again and list the files that differ from what is on disk.
    #[arg(long)]
    pub files: bool,
}

/// The languages `--lang` accepts. One that this `iohr` does not generate yet is
/// hidden from the help and refused with a message, never rendered as a placeholder.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Lang {
    Typescript,
    Python,
    Go,
    Java,
    Csharp,
    Swift,
    Rust,
}

impl Lang {
    /// The generator's language.
    #[must_use]
    pub fn language(self) -> iohr_codegen::Language {
        use iohr_codegen::Language;
        match self {
            Self::Typescript => Language::TypeScript,
            Self::Python => Language::Python,
            Self::Go => Language::Go,
            Self::Java => Language::Java,
            Self::Csharp => Language::CSharp,
            Self::Swift => Language::Swift,
            Self::Rust => Language::Rust,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum BrowserCommand {
    /// Open the page that adds InOrbit Trails to your browser.
    ///
    /// Open the console's install page in the default browser. It adds InOrbit Trails
    /// from that browser's store (Chrome Web Store, Edge Add-ons, Firefox add-ons), or
    /// shows the load-unpacked steps while that store has no listing yet.
    Install(BrowserInstall),
}

#[derive(Debug, Args)]
pub struct BrowserInstall {
    /// Print the page's address instead of opening it (for a browser on another machine).
    #[arg(long)]
    pub print: bool,
}

#[derive(Debug, Subcommand)]
pub enum CoordinatorCommand {
    /// The coordinator's jobs.
    #[command(subcommand)]
    Jobs(CoordinatorJobsCommand),
    /// The coordinator's queued asks.
    #[command(subcommand)]
    Asks(CoordinatorAsksCommand),
    /// The Planner's plans: create one, list yours, show one with its items.
    #[command(subcommand)]
    Plan(CoordinatorPlanCommand),
    /// One item of a plan: add, claim, note, finish, or run a command as the step.
    #[command(subcommand)]
    Item(CoordinatorItemCommand),
    /// Print a job's brief.
    ///
    /// A job's brief, the one text a worker starts from: the task, its acceptance, the
    /// files and the linked document's excerpt, capped at the kind's token budget
    ///. Prints the text alone so it can be piped into a worker.
    Brief {
        /// The job id (`job_...`).
        job: String,
    },
}

#[derive(Debug, Subcommand)]
#[allow(
    clippy::large_enum_variant,
    reason = "parsed once per run; a box would only add noise"
)]
pub enum CoordinatorJobsCommand {
    /// List jobs.
    List {
        /// Only this state: a stage (intake ... done, stopped) or held, retry, awaiting.
        #[arg(long)]
        state: Option<String>,
        /// Only your own jobs.
        #[arg(long)]
        mine: bool,
        /// At most this many.
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=200))]
        limit: u32,
        /// Act on this account: a team you belong to (default: your own).
        #[arg(long)]
        account: Option<String>,
    },
    /// File a job.
    ///
    /// File a job. The brief is the text to work on, from --brief or `-` for standard input.
    Create {
        /// The kind: question, watch, docs, ui, fix, review, chore, mentor, api or design.
        #[arg(long)]
        kind: String,
        /// The brief, or `-` to read it from standard input.
        #[arg(long)]
        brief: String,
        /// The PRD, RFC or ADR it implements.
        #[arg(long)]
        document: Option<String>,
        /// The repository, such as inorbithr/core.
        #[arg(long)]
        repository: Option<String>,
        /// The model, when not the kind's default.
        #[arg(long)]
        model: Option<String>,
        /// Why the model differs from the default (needed with --model).
        #[arg(long)]
        model_override_reason: Option<String>,
        /// A tag; repeat or comma-separate.
        #[arg(long = "tag", value_delimiter = ',')]
        tags: Vec<String>,
        /// Priority, higher first.
        #[arg(long)]
        priority: Option<u32>,
        /// The job this one replaces.
        #[arg(long)]
        supersedes: Option<String>,
        /// Security-relevant: done only with a compliance pull request.
        #[arg(long)]
        security: bool,
        /// The inorbithr/compliance pull request, when it exists.
        #[arg(long)]
        compliance_pr: Option<String>,
        /// The queued ask this job works on.
        #[arg(long)]
        ask: Option<String>,
        /// Act on this account: a team you belong to (default: your own).
        #[arg(long)]
        account: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum AsksCommand {
    /// List the asks waiting for you, live on a terminal.
    List {
        /// open (pending and question, the default), all, or one status: pending, question,
        /// approved, denied, `in_progress`, done, failed, stale.
        #[arg(long, default_value = "open")]
        status: String,
        /// Only this kind: `pull_request`, roll, review, document, decision or run.
        #[arg(long)]
        kind: Option<String>,
        /// Print once and stop, even on a terminal.
        #[arg(long)]
        once: bool,
        /// At most this many.
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=500))]
        limit: u32,
        /// Act on this account: a team you belong to (default: your own).
        #[arg(long)]
        account: Option<String>,
    },
    /// Print how many asks are waiting for you, for a shell prompt.
    ///
    /// Prints "N asks" when some wait, nothing when none do, and `--json` prints `{"count": N}`.
    /// With `--cache` the number (and nothing else) is kept in `~/.inorbit/term/asks-count`,
    /// written whole, and the call does nothing while that file is under 30 seconds old; the
    /// shell hook (`iohr shell hook zsh`) reads that file at the prompt and refreshes it in
    /// the background.
    Count {
        /// Keep the number in `~/.inorbit/term/asks-count`; refresh it at most every 30 seconds.
        #[arg(long)]
        cache: bool,
        /// Act on this account: a team you belong to (default: your own).
        #[arg(long)]
        account: Option<String>,
    },
    /// Show one ask with its timeline, live on a terminal.
    Show {
        /// The ask's id (apr_...).
        id: String,
        /// Print once and stop, even on a terminal.
        #[arg(long)]
        once: bool,
        /// Act on this account: a team you belong to (default: your own).
        #[arg(long)]
        account: Option<String>,
    },
    /// Choose what to do with an ask from a menu: approve, deny or question.
    Decide {
        /// The ask's id (apr_...).
        id: String,
        /// Act on this account: a team you belong to (default: your own).
        #[arg(long)]
        account: Option<String>,
    },
    /// Deny an ask, with a note if you like.
    Deny {
        /// The ask's id (apr_...).
        id: String,
        /// Why (at most 500 characters); the agent reads it.
        #[arg(long)]
        note: Option<String>,
        /// Act on this account: a team you belong to (default: your own).
        #[arg(long)]
        account: Option<String>,
    },
    /// Put a question to the agent that asked; the ask stays open.
    Question {
        /// The ask's id (apr_...).
        id: String,
        /// The question (at most 500 characters); asked here when left out on a terminal.
        #[arg(long)]
        note: Option<String>,
        /// Act on this account: a team you belong to (default: your own).
        #[arg(long)]
        account: Option<String>,
    },
    /// Approve: on a Mac with an enrolled key, after Touch ID; on your phone for a merge, a
    /// roll, a migration or an auto-approve rule, and on every other machine.
    ///
    /// A Mac signs the same decision proof the phone does, with a key in its Secure Enclave
    /// (`iohr asks key enroll`). The terminal prints what is approved and Touch ID shows the
    /// same title. Anything that is hard to take back is refused here and waits for the phone.
    Approve {
        /// The ask's id (apr_...).
        id: String,
        /// Say so and stop, without waiting.
        #[arg(long)]
        once: bool,
        /// Act on this account: a team you belong to (default: your own).
        #[arg(long)]
        account: Option<String>,
    },
    /// The key this Mac approves with: enrol it, look at it, remove it.
    Key {
        #[command(subcommand)]
        command: AsksKeyCommand,
    },
}

/// `iohr asks key`: the Secure Enclave key a Mac approves asks with (RFC 0113).
#[derive(Debug, Subcommand)]
pub enum AsksKeyCommand {
    /// Make this Mac's key (in the Secure Enclave, behind Touch ID) and enrol it.
    ///
    /// With `--proof` from an enrolled phone key the service activates the key at once;
    /// without it the key waits 24 hours (your first key is active at once). Only a Mac
    /// can do this.
    Enroll {
        /// A name for the key (1 to 100 characters); default: this Mac's name.
        #[arg(long)]
        label: Option<String>,
        /// A JSON file with the phone's countersignature of this key:
        /// `{"key_id", "signed_at", "nonce", "signature"}`.
        #[arg(long, value_name = "FILE")]
        proof: Option<std::path::PathBuf>,
    },
    /// Show this Mac's key and what the service knows of it.
    Show,
    /// Revoke this Mac's key at the service and delete it from this Mac.
    Remove,
}

#[derive(Debug, Subcommand)]
pub enum CoordinatorAsksCommand {
    /// List queued asks.
    List {
        /// queued, building, stuck, done, closed, withdrawn or open.
        #[arg(long)]
        state: Option<String>,
        /// Only asks from this source (cli, desktop, mcp, ...).
        #[arg(long)]
        source: Option<String>,
        /// At most this many.
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=200))]
        limit: u32,
        /// Act on this account: a team you belong to (default: your own).
        #[arg(long)]
        account: Option<String>,
    },
    /// Queue an ask.
    ///
    /// Queue an ask. The coordinator's planner files a job for a kind it knows (bug,
    /// feature, ui, api, docs, design, review, chore, question, watch); an ask with
    /// any other kind stays queued, with the reason.
    Create {
        /// What kind of ask: lower case, 1 to 40 of a-z, 0-9, _ and -.
        #[arg(long)]
        kind: String,
        /// The ask, or `-` to read it from standard input.
        #[arg(long)]
        body: String,
        /// A short title; empty is the body's first line.
        #[arg(long)]
        title: Option<String>,
        /// Priority 1 to 10 (default 5).
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=10))]
        priority: Option<u32>,
        /// A link (pull request, document); repeat or comma-separate.
        #[arg(long = "link", value_delimiter = ',')]
        links: Vec<String>,
        /// The id in the system it came from; the same id returns the same ask.
        #[arg(long)]
        external_id: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum CoordinatorPlanCommand {
    /// List the plans you may see.
    List {
        /// active, paused or archived (default: all).
        #[arg(long)]
        state: Option<String>,
        /// At most this many.
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=200))]
        limit: u32,
        /// Act on this account: a team you belong to (default: your own).
        #[arg(long)]
        account: Option<String>,
    },
    /// Show a plan with its items and their latest notes.
    Show {
        /// The plan id (`plan_...`).
        plan: String,
        /// Act on this account: a team you belong to (default: your own).
        #[arg(long)]
        account: Option<String>,
    },
    /// Create a plan you own, or find the one with the same external id.
    Create {
        /// The plan's title, 1 to 200 characters.
        title: String,
        /// The id in the system it comes from; the same id returns the same plan.
        #[arg(long)]
        external_id: Option<String>,
        /// Who sees it: owner (the default) or account (every member).
        #[arg(long, value_parser = ["owner", "account"])]
        access: Option<String>,
        /// Act on this account: a team you belong to (default: your own).
        #[arg(long)]
        account: Option<String>,
    },
}

/// The plan and item every `coordinator item` command names.
#[derive(Debug, Args)]
pub struct PlanItemRef {
    /// The plan id (`plan_...`).
    pub plan: String,
    /// The item's slug in the plan: 1 to 80 of A-Z, a-z, 0-9, `_`, `.` and `-`.
    pub item: String,
    /// Act on this account: a team you belong to (default: your own).
    #[arg(long)]
    pub account: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum CoordinatorItemCommand {
    /// Add an item to a plan (or change its title, kind and text), as proposed.
    Add {
        #[command(flatten)]
        at: PlanItemRef,
        /// What the item is called, 1 to 200 characters.
        #[arg(long)]
        title: String,
        /// What kind of item: feature, bug, docs, ... (default: task).
        #[arg(long)]
        kind: Option<String>,
        /// A longer description, or `-` to read it from standard input.
        #[arg(long)]
        body: Option<String>,
    },
    /// Claim an item before work on it starts: a lease held by one holder.
    Claim {
        #[command(flatten)]
        at: PlanItemRef,
        /// Who builds it, 1 to 120 characters (default: iohr and your profile).
        #[arg(long)]
        holder: Option<String>,
        /// How long the lease lasts, 1 to 86400 (default 3600).
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=86_400))]
        seconds: Option<u32>,
        /// Take it over from another holder's live claim.
        #[arg(long)]
        force: bool,
    },
    /// Give a claim up.
    Release {
        #[command(flatten)]
        at: PlanItemRef,
        /// The holder (default: iohr and your profile).
        #[arg(long)]
        holder: Option<String>,
        /// Release another holder's live claim.
        #[arg(long)]
        force: bool,
    },
    /// Write a note on an item.
    Note {
        #[command(flatten)]
        at: PlanItemRef,
        /// The note, 1 to 2000 characters, or `-` to read it from standard input.
        text: String,
    },
    /// Finish an item: an optional closing note, then status done (this clears the claim).
    Done {
        #[command(flatten)]
        at: PlanItemRef,
        /// A closing note.
        #[arg(long)]
        note: Option<String>,
        /// Who is writing (default: iohr and your profile).
        #[arg(long)]
        holder: Option<String>,
    },
    /// Fail an item: a `failed:` note, status back to proposed and the claim released.
    ///
    /// The Planner has no failed status, and dropped is final, so a failed step goes back
    /// to proposed with the reason in its notes, ready to be claimed again.
    Fail {
        #[command(flatten)]
        at: PlanItemRef,
        /// Why it failed.
        #[arg(long)]
        reason: String,
        /// Who is writing (default: iohr and your profile).
        #[arg(long)]
        holder: Option<String>,
    },
    /// Run a command as a Planner step: claim, building, the command, then done or failed.
    ///
    /// Adds the item when the plan lacks it, skips one that is already done, refuses one
    /// another holder is building, and ends with a note and the status the command's exit
    /// earned. The command's exit code is this command's (1 when it could not start).
    Run {
        #[command(flatten)]
        at: PlanItemRef,
        /// The title when the item has to be added (default: the item's slug).
        #[arg(long)]
        title: Option<String>,
        /// The kind when the item has to be added (default: task).
        #[arg(long)]
        kind: Option<String>,
        /// Who builds it (default: iohr and your profile).
        #[arg(long)]
        holder: Option<String>,
        /// The command and its arguments, after `--`.
        #[arg(last = true, required = true, num_args = 1..)]
        command: Vec<String>,
    },
}

#[derive(Debug, Args)]
pub struct Tunnel {
    /// The ssh target that runs the environment.
    #[arg(long, env = "IOHR_TUNNEL_HOST")]
    pub host: String,
    /// The port to listen on here, on 127.0.0.1 (default: the edge's own port).
    #[arg(long)]
    pub port: Option<u16>,
    /// The edge port on the host, when it differs from `environments.toml` (core's
    /// `devops/ci/envs.toml` is the source; `mise run local:tunnel` passes it).
    #[arg(long)]
    pub edge_port: Option<u16>,
    /// The engine LB port to forward as well (default: the environment's, when it has
    /// one: mobile and Trails use it); the same port on both ends.
    #[arg(long)]
    pub engine_port: Option<u16>,
    /// Forward the edge only, not the engine LB.
    #[arg(long)]
    pub no_engine: bool,
    /// Print the ssh command and the addresses, then stop.
    #[arg(long)]
    pub dry_run: bool,
}

#[cfg(test)]
mod grammar_tests {
    use super::GROUPS;
    use clap::CommandFactory as _;

    #[test]
    fn every_visible_command_is_in_exactly_one_help_group() {
        // `command()` hides the subcommands from clap's own list; the names come from the
        // un-hidden derive tree.
        let tree = super::Cli::command();
        let visible: Vec<&str> = tree
            .get_subcommands()
            .filter(|c| !c.is_hide_set())
            .map(clap::Command::get_name)
            .collect();
        assert!(visible.len() > 10, "{visible:?}");
        for name in &visible {
            let n = GROUPS.iter().filter(|(_, g)| g.contains(name)).count();
            assert_eq!(n, 1, "`{name}` is in {n} help groups; it needs exactly one");
        }
        for (title, names) in GROUPS {
            for name in *names {
                assert!(
                    *name == "help" || visible.contains(name),
                    "group {title} lists `{name}`, which is not a visible command"
                );
            }
        }
    }

    #[test]
    fn read_repeats_and_add_dir_is_the_same_flag() {
        use clap::Parser as _;
        let cli = super::Cli::try_parse_from([
            "iohr",
            "ask",
            "--read",
            "/a",
            "--add-dir",
            "/b",
            "--add-dir=/c",
            "what is in them",
        ])
        .unwrap();
        let super::Command::Ask(a) = cli.command else {
            panic!("not ask")
        };
        let got: Vec<_> = a
            .headless
            .read
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        assert_eq!(got, ["/a", "/b", "/c"]);
        assert!(a.headless.has_context());
        // Not given: no folders and no context of its own.
        let cli = super::Cli::try_parse_from(["iohr", "ask", "hi"]).unwrap();
        let super::Command::Ask(a) = cli.command else {
            panic!("not ask")
        };
        assert_eq!(a.headless.read.len(), 0);
    }

    #[test]
    fn the_old_names_parse() {
        use clap::Parser as _;
        let p = |a: &[&str]| {
            super::Cli::try_parse_from(std::iter::once("iohr").chain(a.iter().copied()))
        };
        assert!(p(&["packages"]).is_ok());
        assert!(p(&["project", "templates"]).is_ok());
        assert!(
            p(&["project", "template", "lint"]).is_ok(),
            "lint defaults to ."
        );
    }
}
