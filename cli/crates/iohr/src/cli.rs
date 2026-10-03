//! The command-line surface: every command, flag and environment variable.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use iohr_auth::ProfileName;

use crate::api::DEFAULT_BASE_URL;

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
#[derive(Debug, Args)]
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
    /// Print a shell completion script.
    Completion {
        /// The shell.
        shell: Shell,
    },
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
    /// Write a surface for one or more documents into a directory, with its `iohr.lock`.
    Generate(SdkGenerate),
}

#[derive(Debug, Args)]
pub struct SdkGenerate {
    /// The language.
    #[arg(long, value_enum)]
    pub lang: Lang,
    /// A document to generate from, as NAME=FILE: the profile's name and the document it
    /// saw (`iohr openapi pull`). Repeat it for several profiles.
    #[arg(long = "from", value_name = "NAME=FILE", required = true)]
    pub from: Vec<String>,
    /// The directory to write the surface into; `iohr.lock` goes beside it.
    #[arg(long, value_name = "DIR")]
    pub out: PathBuf,
    /// Replace a non-empty output directory.
    #[arg(long)]
    pub force: bool,
    #[command(flatten)]
    pub rust: iohr_codegen::RustOptions,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Lang {
    Rust,
}
