//! One module per command.

mod accounts;
mod auth;
mod browser_ext;
mod call;
mod config;
mod connections;
mod connectors;
mod decisions;
mod domains;
mod ext;
mod ext_catalogue;
mod lab;
mod login;
mod openapi;
mod profile;
mod sdk;
mod sdk_add;
mod token;
mod whoami;

use clap::CommandFactory as _;

use crate::Env;
use crate::cli::{
    AccountsCommand, AuthCommand, BrowserCommand, Cli, Command, LabCommand, OpenapiCommand,
    SdkCommand,
};
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
        Command::Profile(cmd) => profile::run(g, &env, cmd, out).await,
        Command::Whoami => whoami::run(g, &env, out).await,
        Command::Auth(AuthCommand::Token(args)) => auth::token(g, &env, &args, out).await,
        Command::Accounts(AccountsCommand::List) => accounts::list(g, &env, out).await,
        Command::Token(cmd) => token::run(g, &env, cmd, out).await,
        Command::Api(call) => call::run(g, &env, call, out).await,
        Command::Openapi(OpenapiCommand::Pull { output }) => {
            openapi::pull(g, &env, &output, out).await
        }
        Command::Sdk(SdkCommand::Generate(args)) => sdk::generate(g, &env, &args, out).await,
        Command::Sdk(SdkCommand::Check(args)) => sdk::check(g, &env, &args, out).await,
        Command::Sdk(SdkCommand::Examples(args)) => sdk::examples(&args, out),
        Command::Sdk(SdkCommand::Config(args)) => sdk::config(g, &args),
        Command::Sdk(SdkCommand::Add(args)) => sdk_add::add(&args, out),
        Command::Domains(cmd) => domains::run(g, &env, cmd, out).await,
        Command::Connectors(cmd) => connectors::run(g, &env, cmd, out).await,
        Command::Connections(cmd) => connections::run(g, &env, cmd, out).await,
        Command::Ext(cmd) => ext::run(g, &env, cmd, out).await,
        Command::Config(cmd) => config::run(g, cmd, out),
        Command::Lab(LabCommand::Check(args)) => lab::check(&args, out),
        Command::Decisions(cmd) => decisions::run(g, &env, cmd, out).await,
        Command::Browser(BrowserCommand::Install(args)) => {
            browser_ext::install(g, &args, out);
            Ok(())
        }
        Command::External(argv) => ext::external(g, &env, argv).await,
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

/// The items of every page of a list (AIP-158), until the last page or `limit` items.
pub(crate) async fn pages(
    api: &crate::context::Api,
    path: &str,
    query: &[(&str, &str)],
    items: &str,
    limit: usize,
) -> Result<Vec<serde_json::Value>, inorbithr::Error> {
    let mut all: Vec<serde_json::Value> = Vec::new();
    let mut page_token = String::new();
    let page_size = limit.clamp(1, 200).to_string();
    loop {
        let mut q: Vec<(&str, &str)> = query.to_vec();
        q.push(("page_size", &page_size));
        if !page_token.is_empty() {
            q.push(("page_token", &page_token));
        }
        let page: serde_json::Value = api.get(path, &q).await?;
        all.extend(
            page.get(items)
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default(),
        );
        page.get("next_page_token")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .clone_into(&mut page_token);
        if page_token.is_empty() || all.len() >= limit {
            break;
        }
    }
    all.truncate(limit);
    Ok(all)
}

/// A string field of a JSON object; `""` when it is missing or not a string.
pub(crate) fn str_of<'a>(v: &'a serde_json::Value, k: &str) -> &'a str {
    v.get(k)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}
