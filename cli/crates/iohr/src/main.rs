//! The `iohr` binary: parse, run, map the outcome to an exit code.

use std::io::Write as _;
use std::process::ExitCode;

use clap::Parser as _;
use iohr::cli::Cli;
use iohr::{Env, refuse_secrets_in_args, run};

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().collect();
    if let Err(e) = refuse_secrets_in_args(&args) {
        return fail(&e);
    }
    let cli = Cli::parse_from(args);
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
