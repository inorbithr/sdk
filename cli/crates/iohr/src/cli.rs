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
pub enum ExtCommand {
    /// Fetch an extension, verify its signature, provenance and digest, and install it.
    Install {
        /// NAME, NAME@VERSION or NAME@sha256:DIGEST (default: the newest release).
        extension: String,
        /// Also pin it in this lock file, for `iohr ext sync` elsewhere.
        #[arg(long, value_name = "FILE")]
        lock: Option<PathBuf>,
    },
    /// The installed extensions.
    List,
    /// Install the newest release of one extension, or of every installed one.
    Upgrade {
        /// The extension (default: all).
        name: Option<String>,
        /// Also pin the result in this lock file.
        #[arg(long, value_name = "FILE")]
        lock: Option<PathBuf>,
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
    #[value(hide = true)]
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
