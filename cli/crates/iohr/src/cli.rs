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
    version,
    about,
    long_about,
    propagate_version = true,
    max_term_width = 100
)]
pub struct Cli {
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub command: Command,
}

/// Flags every command takes.
#[derive(Debug, Clone, Args)]
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
    /// The environment to call: `production` (the default) or `local`. Sets the API and
    /// sign-in addresses; `--base-url` and `IOHR_ISSUER` still win over it.
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
    /// Where profiles are kept (default: the platform's config directory).
    #[arg(long, global = true, env = "IOHR_CONFIG_DIR", hide = true)]
    pub config_dir: Option<PathBuf>,
    /// Where extensions are kept (default: the platform's data directory).
    #[arg(long, global = true, env = "IOHR_DATA_DIR", hide = true)]
    pub data_dir: Option<PathBuf>,
}

impl Global {
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

#[derive(Debug, Subcommand)]
pub enum Command {
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
    /// The coordinator's jobs and queued asks: list and file work (RFC 0120, RFC 0138).
    #[command(subcommand)]
    Coordinator(CoordinatorCommand),
    /// The apps a connection can be made from: incident tools, chat, observability, AI models
    /// (RFC 0044).
    #[command(subcommand)]
    Connectors(ConnectorsCommand),
    /// The account's connections to outside systems: add one with a key or by signing
    /// in, test, pause, grant its actions, delete it (RFC 0044).
    #[command(subcommand)]
    Connections(ConnectionsCommand),
    /// Extensions: separate programs iohr installs from a registry, verifies, pins and
    /// runs as `iohr <name> ...`.
    #[command(subcommand)]
    Ext(ExtCommand),
    /// Every package you have in one table: published by your account (public or
    /// private), private ones shared with it, unpublished ones in progress on this
    /// machine, and what is installed. The same as `iohr ext packages`.
    Packages,
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
    /// Lab documents: RFCs and studies kept in your repository (RFC 0035).
    #[command(subcommand)]
    Lab(LabCommand),
    /// Decisions: RFCs, PRDs and ADRs in the Decisions product (RFC 0065, core ADR 0056):
    /// list and read them, create one, save a new version, set its status, comment, link a
    /// pull request, ask for a review.
    #[command(subcommand)]
    Decisions(DecisionsCommand),
    /// A project you build yourself (PRD 0088): one command sets up everything it needs,
    /// from sign-in to a smoke test, and reports what you now have.
    #[command(subcommand)]
    Project(ProjectCommand),
    /// The coordinator's runner (RFC 0120): this machine runs coordinator turns on your own
    /// installed `claude`, on your own subscription.
    #[command(subcommand)]
    Runner(RunnerCommand),
    /// InOrbit Trails, the browser extension: open the page that adds it to your browser.
    #[command(subcommand)]
    Browser(BrowserCommand),
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
    /// `expires_at` (RFC 3339, or null when the token does not expire), `profile` (null
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
    /// The kind of document (RFC 0081): `rfc` (default), `prd`, `adr` or `study`; `all`
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
    /// Register this machine as one of your runners: its name, system and what it can run.
    ///
    /// Needs `claude` on your PATH, signed in with your subscription (`claude`, then
    /// `/login`). Nothing about the sign-in leaves the machine except a hash of the line
    /// `claude auth status` prints. Run it again and it says the runner exists.
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
        #[command(flatten)]
        account: AccountArg,
    },
    /// Run the loop: heartbeat, accept a lease, run its turn on `claude -p`, report it.
    /// Stops on Ctrl-C.
    Run {
        /// The runner's name (default: this machine's host name).
        #[arg(long, conflicts_with = "runner")]
        name: Option<String>,
        /// The runner's id, from `iohr runner status`.
        #[arg(long)]
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
        #[command(flatten)]
        account: AccountArg,
    },
    /// The runner's settings from config.toml: `[runner]` and `[profiles.<name>.runner]`.
    #[command(subcommand)]
    Config(RunnerConfigCommand),
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
        /// The runner's id, from `iohr runner status`.
        #[arg(long)]
        runner: Option<String>,
        #[command(flatten)]
        account: AccountArg,
    },
    /// Your runners as the service sees them, and what this machine can run.
    Status {
        #[command(flatten)]
        account: AccountArg,
    },
}

#[derive(Debug, Subcommand)]
pub enum RunnerConfigCommand {
    /// The values `iohr runner run` would use now, each with where it came from: a flag,
    /// `[profiles.<name>.runner]`, `[runner]` or the default. Needs no sign-in.
    Show {
        /// As `iohr runner run --capacity`.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=32))]
        capacity: Option<u32>,
        /// As `iohr runner run --kinds`.
        #[arg(long, value_delimiter = ',')]
        kinds: Option<Vec<String>>,
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
    /// A new RFC, PRD or ADR in a space: the account gives it the next number of its
    /// kind. With `--file`, the file's text is saved as its next version straight away.
    Create {
        /// The space, by slug (`platform`) or id.
        #[arg(long)]
        space: String,
        /// The kind (RFC 0081): `rfc` (default), `prd` or `adr`.
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
    /// Set a document's status in the words of its kind (RFC 0081): an RFC's `draft`,
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
    /// One connection: who it is at the provider, its health and grants. Never shows
    /// a credential.
    Show {
        /// The connection's name or id (con_...).
        connection: String,
        #[command(flatten)]
        account: AccountArg,
    },
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
    /// Give a connection a new credential: a key is asked for again, a signed-in
    /// connection opens the provider in a browser.
    Reconnect(ConnectionsReconnect),
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
    /// Every package you have, in one table: NAME, KIND, SOURCE (account, shared, local,
    /// installed), VISIBILITY, VERSION and INSTALLED. Published ones, private too, come
    /// from the API; local ones are the folders under ~/.inorbit/ext-dev, read from disk
    /// and never sent anywhere. An installed extension is matched to a package by name
    /// alone, since the lock keeps no publisher. With `--json`, an array of the same fields.
    Packages,
    /// Search the extensions catalogue (RFC 0073): name, kind, publisher, latest
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
    /// One extension in the catalogue: what it does, its publisher, signer, scopes,
    /// privileges in plain words, evidence, versions and the line that installs it.
    Show {
        /// PUBLISHER/NAME, such as inorbit/agent.
        extension: String,
    },
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
    /// Check installed extensions again, offline: the program's hash, the signature
    /// and provenance kept at install, and the signer the lock pins.
    Verify {
        /// The extension (default: all).
        name: Option<String>,
    },
    /// Install exactly what a lock file pins, by digest and signer.
    Sync {
        /// The lock file.
        #[arg(long, default_value = "iohr-ext.lock", value_name = "FILE")]
        lock: PathBuf,
    },
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
    /// Check the extension.toml in a folder: unknown keys, values that look like
    /// secrets, data outside ADR 0063's non-sensitive kinds, unsupported targets, a
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

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// Set up a project of your own: sign-in checked, the folder scaffolded from a
    /// template with its manifest, a git repository with one commit, its own state under
    /// ~/.inorbit/projects/NAME/, a Keychain slot by name, a smoke test that runs it once,
    /// and a report. Idempotent: a step already done is done again without harm, and a
    /// step that waits on an API says so instead of pretending.
    Init {
        /// The project's name: lowercase letters, digits and dashes, like demo or my-shop.
        name: String,
        /// The template: demo (a private macOS service that heartbeats and serves a tiny
        /// panel). Others say they are coming.
        #[arg(long, default_value = "demo")]
        template: String,
        /// The folder to write (default: ~/development/NAME); it must not exist or be empty.
        #[arg(long, value_name = "DIR")]
        dir: Option<PathBuf>,
        /// The publisher half of the extension id (default: your account's slug, else
        /// "local"); the private listing takes it when that step lands.
        #[arg(long)]
        publisher: Option<String>,
        /// Skip the smoke test (it runs the project once with python3).
        #[arg(long)]
        no_smoke: bool,
    },
}

/// What `iohr ext init` scaffolds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InitKind {
    /// A program that runs as its own process (RFC 0073).
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
    /// Write a surface cut to what your profiles may call into a directory, with its
    /// `iohr.lock` beside it.
    Generate(SdkGenerate),
    /// Fetch every profile's document again and fail when the cut has moved since the
    /// surface was generated (run it in CI with `IOHR_TOKEN_<PROFILE>` per profile).
    Check(SdkCheck),
    /// Write a short program per operation and language that calls it with the published
    /// runtime, as JSON keyed by operation id: what the API reference shows beside each
    /// operation. Every snippet builds against the runtime's public surface.
    Examples(SdkExamples),
    /// Print the configuration an SDK client built with `load` would use here: every
    /// setting with where its value came from, the credential chain, the pipeline and
    /// what was ignored, as the SDKs' `describe()` gives it (`docs/config.md` section
    /// 2.6). Secrets are redacted. Reads the environment (`INORBIT_*`) and the config
    /// file; contacts no host. `--profile NAME` stands for `profile` in the client's
    /// code; `IOHR_PROFILE` is not read, as the SDKs do not read it.
    Config(SdkConfig),
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
            Self::Rust => Language::Rust,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum BrowserCommand {
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
    /// A job's brief, the one text a worker starts from: the task, its acceptance, the
    /// files and the linked document's excerpt, capped at the kind's token budget
    /// (RFC 0132). Prints the text alone so it can be piped into a worker.
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
