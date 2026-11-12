//! The program: `run` with the real streams, and its result as the exit code.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ExitCode::from(@@MODULE@@::run(
        &args,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
        @@MODULE@@::COMMIT,
    ))
}
