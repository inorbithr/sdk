//! The command-line surface: every command, flag and environment variable.

use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use iohr_auth::ProfileName;

use inorbithr::DEFAULT_BASE_URL;

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
    /// The API's address.
    #[arg(long, global = true, env = "IOHR_BASE_URL", default_value = DEFAULT_BASE_URL, hide_default_value = true)]
    pub base_url: String,
    /// The sign-in service.
    #[arg(long, global = true, env = "IOHR_ISSUER", default_value = iohr_auth::DEFAULT_ISSUER, hide = true)]
    pub issuer: String,
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
    /// Settings besides profiles: where extensions come from, which keys may sign them.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Lab documents: RFCs and studies kept in your repository (RFC 0035).
    #[command(subcommand)]
    Lab(LabCommand),
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
    /// Fetch an extension, verify its signature, provenance and digest, and install it.
    /// One that declares privileges for its system service asks first unless --yes.
    Install {
        /// NAME, NAME@VERSION or NAME@sha256:DIGEST (default: the newest release).
        extension: String,
        /// Also pin it in this lock file, for `iohr ext sync` elsewhere.
        #[arg(long, value_name = "FILE")]
        lock: Option<PathBuf>,
        /// Confirm the privileges it declares without asking (needed without a terminal).
        #[arg(long, short = 'y')]
        yes: bool,
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
