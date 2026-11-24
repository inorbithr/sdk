//! The `iohr` binary: parse, run, map the outcome to an exit code.

use std::process::ExitCode;

use clap::FromArgMatches as _;
use iohr::cli::Cli;
use iohr::{Env, refuse_secrets_in_args, run};

// iohr's command tree and the command future are large, and Windows gives the main thread
// 1 MiB of stack (Linux gives 8 MiB): parsing alone overflowed it on windows-latest. main
// only starts a thread with a stack every platform can hold; everything runs there.
fn main() -> ExitCode {
    let worker = std::thread::Builder::new()
        .name("iohr".into())
        .stack_size(STACK_BYTES)
        .spawn(start);
    match worker.map(std::thread::JoinHandle::join) {
        Ok(Ok(code)) => code,
        Ok(Err(panic)) => std::panic::resume_unwind(panic),
        Err(e) => {
            let json = std::env::args_os().any(|a| a == "--json");
            iohr::report_failure(json, &iohr::Error::Failed(format!("cannot start: {e}")))
        }
    }
}

/// The stack iohr runs on: 16 MiB, room above Linux's 8 MiB main thread default.
const STACK_BYTES: usize = 16 * 1024 * 1024;

fn start() -> ExitCode {
    let args: Vec<_> = std::env::args_os().collect();
    let json = args.iter().any(|a| a == "--json");
    if let Err(e) = refuse_secrets_in_args(&args) {
        return iohr::report_failure(json, &e);
    }
    let parsed = iohr::cli::command(json)
        .try_get_matches_from(&args)
        .and_then(|matches| Cli::from_arg_matches(&matches));
    let cli = match parsed {
        Ok(cli) => cli,
        Err(e) => return iohr::report_usage_clap(&args, json, &e),
    };
    if cli.command.is_none() && iohr::bare_inside(&cli.global) {
        return ExitCode::SUCCESS;
    }
    if cli.command.is_none() && !iohr::bare_enters(&cli.global) {
        // Bare `iohr` with no terminal, or under NO_COLOR or --json: the usage report, as before.
        let e = iohr::cli::command(json).error(
            clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand,
            "",
        );
        return iohr::report_usage_clap(&args, json, &e);
    }
    run_command(&args, json, cli)
}

fn run_command(args: &[std::ffi::OsString], json: bool, cli: Cli) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            return iohr::report_failure(json, &iohr::Error::Failed(format!("cannot start: {e}")));
        }
    };
    match runtime.block_on(run(cli, Env::from_process())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(iohr::Error::Usage(message)) if !message.is_empty() => {
            iohr::report_usage_message(args, json, &message)
        }
        Err(e) => iohr::report_failure(json, &e),
    }
}
