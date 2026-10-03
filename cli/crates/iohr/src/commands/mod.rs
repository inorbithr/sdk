//! One module per command.

mod accounts;
mod call;
mod login;
mod openapi;
mod profile;
mod sdk;
mod token;
mod whoami;

use clap::CommandFactory as _;

use crate::Env;
use crate::cli::{AccountsCommand, Cli, Command, OpenapiCommand, SdkCommand};
use crate::error::Error;
use crate::output::Out;

/// Runs the parsed command.
///
/// # Errors
///
/// [`Error`], whose [`exit_code`](Error::exit_code) the binary returns.
pub async fn run(cli: Cli, env: Env) -> Result<(), Error> {
    let out = Out {
        json: cli.global.json,
    };
    let g = &cli.global;
    match cli.command {
        Command::Login(args) => login::login(g, &env, &args, out).await,
        Command::Logout => login::logout(g, out).await,
        Command::Profile(cmd) => profile::run(g, cmd, out).await,
        Command::Whoami => whoami::run(g, &env, out).await,
        Command::Accounts(AccountsCommand::List) => accounts::list(g, &env, out).await,
        Command::Token(cmd) => token::run(g, &env, cmd, out).await,
        Command::Api(call) => call::run(g, &env, call, out).await,
        Command::Openapi(OpenapiCommand::Pull { output }) => {
            openapi::pull(g, &env, &output, out).await
        }
        Command::Sdk(SdkCommand::Generate(args)) => sdk::generate(&args, out),
        Command::Completion { shell } => {
            let mut buf = Vec::new();
            clap_complete::generate(shell, &mut Cli::command(), "iohr", &mut buf);
            Out::raw(&buf);
            Ok(())
        }
    }
}

/// `2026-10-09` from an RFC 3339 timestamp; `""` (unset) and unreadable values stay as
/// they are.
pub(crate) fn day(ts: &str) -> String {
    ts.get(..10)
        .filter(|d| d.len() == 10 && d.as_bytes()[4] == b'-')
        .map_or_else(|| ts.to_owned(), str::to_owned)
}
