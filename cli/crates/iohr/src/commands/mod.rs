//! One module per command.

mod accounts;
mod ask;
mod ask_roles;
pub(crate) mod asks;
mod atlas;
mod auth;
mod browser_ext;
mod call;
mod config;
mod connections;
mod connectors;
pub(crate) mod coordinator;
mod coordinator_plan;
mod decisions;
mod domains;
mod ext;
mod ext_catalogue;
mod ext_packages;
mod folder_context;
pub(crate) mod incident_settings;
mod init_crawl;
mod init_extensions;
mod lab;
mod login;
mod memory;
mod openapi;
mod permissions;
mod profile;
mod project;
mod project_plan;
mod project_space;
mod project_template;
mod runner;
mod runner_ops;
mod sdk;
mod sdk_add;
pub(crate) mod session;
pub(crate) mod shell;
mod theme;
mod token;
mod tunnel;
pub(crate) mod upgrade;
mod web_ui;
mod whoami;

use clap::CommandFactory as _;

use crate::Env;
use crate::cli::{
    AccountsCommand, AuthCommand, BrowserCommand, Cli, Command, ExtCommand, LabCommand,
    OpenapiCommand, ProjectCommand, SdkCommand,
};
use crate::error::Error;
use crate::output::Out;

/// Commands that never call the API, so a placeholder address is no bar: a tunnel reaches
/// an environment over ssh, the UI serves files on loopback, the permission commands
/// answer on this machine, and `sign-binary` signs a file on this Mac.
fn calls_no_api(command: Option<&Command>) -> bool {
    matches!(
        command,
        Some(
            Command::Tunnel(_)
                | Command::Ui(_)
                | Command::Permissions(_)
                | Command::PermissionServer { .. }
                | Command::SignBinary { .. }
        )
    )
}

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
    // The permission server speaks MCP on stdout for one `iohr ask` run: it calls no API and
    // must print nothing else.
    #[cfg(unix)]
    if let Some(Command::PermissionServer { socket }) = &cli.command {
        return crate::permissions::server::run(socket)
            .map_err(|e| Error::Failed(format!("permission server: {}", e.kind())));
    }
    if !calls_no_api(cli.command.as_ref()) {
        g.check_environment()?;
    }
    if g.sign_in.is_some()
        && !matches!(
            cli.command,
            Some(Command::Runner(_) | Command::Coordinator(_))
        )
    {
        return Err(Error::Usage(
            "--sign-in (IOHR_SIGN_IN) applies to `iohr runner` and `iohr coordinator` only".into(),
        ));
    }
    if g.sign_in.is_none()
        && (g.via_role().is_some() || g.via_session.is_some() || g.via_job.is_some())
    {
        return Err(Error::Usage(
            "--via-role, --via-session and --via-job need --sign-in io".into(),
        ));
    }
    // Bare `iohr` (the binary has already sent a non-terminal to the usage report).
    let Some(command) = cli.command else {
        return if crate::panel::bare_enters(g) {
            session::enter(g, env).await
        } else {
            Err(Error::Usage(String::new()))
        };
    };
    match command {
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
        Command::Coordinator(cmd) => coordinator::run(g, &env, cmd, out).await,
        Command::Context(args) => folder_context::run(g, &args, out),
        Command::Memory(cmd) => memory::run(g, &env, cmd, out).await,
        Command::Connectors(cmd) => connectors::run(g, &env, cmd, out).await,
        Command::Connections(cmd) => connections::run(g, &env, cmd, out).await,
        Command::Ext(cmd) => ext::run(g, &env, cmd, out).await,
        Command::Packages => ext::run(g, &env, ExtCommand::Packages, out).await,
        Command::Upgrade { check } => upgrade::run(g, &env, check, out).await,
        Command::SignBinary { path } => upgrade::sign_binary(&env, &path),
        Command::Ask(args) => ask::run(g, &env, *args, out).await,
        Command::Asks(cmd) => asks::run(g, &env, cmd, out).await,
        Command::Shell(cmd) => shell::run(g, &cmd),
        Command::Theme(cmd) => theme::run(cmd, out),
        Command::Panel(args) => crate::panel::run(g, env, args).await,
        Command::Permissions(cmd) => permissions::run(g, cmd, out),
        Command::PermissionServer { .. } => Err(Error::Failed(
            "the permission server needs a unix system".to_owned(),
        )),
        Command::Project(cmd) => project::run(g, &env, cmd, out).await,
        Command::Init(args) => project::run(g, &env, ProjectCommand::Init(args), out).await,
        Command::Atlas(cmd) => atlas::run(g, &env, cmd, out).await,
        Command::Sync(args) => atlas::sync(g, &env, &args, out).await,
        Command::Config(cmd) => config::run(g, cmd, out),
        Command::Lab(LabCommand::Check(args)) => lab::check(&args, out),
        Command::Decisions(cmd) => decisions::run(g, &env, cmd, out).await,
        Command::Runner(cmd) => Box::pin(runner::run(g, &env, cmd, out)).await,
        Command::Session(args) => session::open(g, &args, out).await,
        Command::Stop(args) => session::stop(g, &args, out).await,
        Command::Browser(BrowserCommand::Install(args)) => {
            browser_ext::install(g, &args, out);
            Ok(())
        }
        Command::Tunnel(args) => tunnel::run(g, &args, out),
        Command::Ui(args) => web_ui::run(g, &env, args, out).await,
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

/// The first line of `text` for a table cell: control characters (an escape sequence
/// from an agent's text must not reach a terminal) dropped, cut to `max` characters with
/// `...`.
pub(crate) fn headline(text: &str, max: usize) -> String {
    let clean: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();
    let line = clean.lines().next().unwrap_or_default();
    if line.chars().count() > max {
        let cut: String = line.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}...")
    } else {
        line.to_owned()
    }
}

/// A string field of a JSON object; `""` when it is missing or not a string.
pub(crate) fn str_of<'a>(v: &'a serde_json::Value, k: &str) -> &'a str {
    v.get(k)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::calls_no_api;
    use crate::cli::Cli;

    /// `sign-binary` signs a file and calls no API, so a placeholder environment does not stop
    /// it; a command that does call the API is still held to a real address.
    #[test]
    fn sign_binary_is_exempt_from_the_environment_check_like_its_siblings() {
        let parse = |args: &[&str]| {
            let mut v = vec!["iohr"];
            v.extend_from_slice(args);
            Cli::try_parse_from(v).unwrap().command
        };
        assert!(calls_no_api(parse(&["sign-binary", "/tmp/iohr"]).as_ref()));
        assert!(!calls_no_api(parse(&["whoami"]).as_ref()));
    }
}
