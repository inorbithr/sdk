//! `@@NAME@@`: an offline eval suite with a pass bar. Exit 0 when the bar is met, 1 when it is
//! missed, 2 when the dataset or the bar is wrong.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = @@MODULE@@::suite(&args, &mut std::io::stdout(), &mut std::io::stderr(), None);
    ExitCode::from(code)
}
