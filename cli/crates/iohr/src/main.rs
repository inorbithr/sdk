//! The `iohr` binary: parse, run, map the outcome to an exit code.

use std::io::Write as _;
use std::process::ExitCode;

use clap::FromArgMatches as _;
use iohr::cli::Cli;
use iohr::{Env, refuse_secrets_in_args, run};

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().collect();
    if let Err(e) = refuse_secrets_in_args(&args) {
        return fail(&e);
    }
    let json = args.iter().any(|a| a == "--json");
    let cli = match iohr::cli::command(json).try_get_matches_from(args) {
        Ok(matches) => Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit()),
        Err(e) => e.exit(),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return fail(&iohr::Error::Failed(format!("cannot start: {e}"))),
    };
    match runtime.block_on(run(cli, Env::from_process())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}

fn fail(e: &iohr::Error) -> ExitCode {
    let message = e.to_string();
    if !message.is_empty() {
        let _ = writeln!(std::io::stderr().lock(), "iohr: {message}");
    }
    e.exit_code()
}
