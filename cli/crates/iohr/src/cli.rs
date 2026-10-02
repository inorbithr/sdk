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
    /// Where profiles are kept (default: the platform's config directory).
    #[arg(long, global = true, env = "IOHR_CONFIG_DIR", hide = true)]
    pub config_dir: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Sign in and add a profile.
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
    /// Print a shell completion script.
    Completion {
        /// The shell.
        shell: Shell,
    },
}

#[derive(Debug, Args)]
pub struct Login {
    /// Read an API token from standard input.
    #[arg(long)]
    pub with_token: bool,
    /// Keep the credential in an owner-only file instead of the OS credential store.
    #[arg(long)]
    pub insecure_storage: bool,
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
