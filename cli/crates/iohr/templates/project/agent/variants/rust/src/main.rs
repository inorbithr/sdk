//! `cargo run -- "your task"`: runs the loop with the scripted model and prints the outcome,
//! the stop reason and what it used.

use std::process::ExitCode;

use @@MODULE@@::model::{STUB, demo};
use @@MODULE@@::tools::{allow, tools};
use @@MODULE@@::{Agent, Budget, Stop};

fn main() -> ExitCode {
    let given = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let task = if given.is_empty() { "add 2 and 3".to_owned() } else { given };
    let mut agent = Agent {
        model: Box::new(demo()),
        tools: tools(),
        allow: allow(),
        budget: Budget::default(),
        approve: None,
        log: Some(Box::new(std::io::stderr())),
    };
    let run = agent.run(&task);
    println!("[{STUB}]");
    match &run.text {
        Some(text) => println!("{text}"),
        None => println!("(stopped: {})", run.stop.name()),
    }
    println!(
        "stop={} steps={} tool_calls={} tokens={}",
        run.stop.name(),
        run.steps,
        run.tool_calls,
        run.tokens
    );
    if run.stop == Stop::Done { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
