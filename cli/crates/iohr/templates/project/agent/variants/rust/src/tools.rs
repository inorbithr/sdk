//! Your tools. Two examples: one that is safe to run, one that changes something and so needs
//! an answer first. Replace them; keep `needs_approval` on anything that writes, sends or
//! deletes.

use std::collections::{HashMap, HashSet};

use crate::{Arg, Tool};

fn add(args: &HashMap<String, Arg>) -> Result<String, String> {
    match (args.get("a"), args.get("b")) {
        (Some(Arg::Int(a)), Some(Arg::Int(b))) => Ok((a + b).to_string()),
        _ => Err("add takes integers a and b".into()),
    }
}

fn delete_note(args: &HashMap<String, Arg>) -> Result<String, String> {
    // An example of a tool that changes something: it only pretends to here.
    match args.get("id") {
        Some(Arg::Str(id)) => Ok(format!("deleted {id}")),
        _ => Err("delete_note takes an id".into()),
    }
}

#[must_use]
pub fn tools() -> Vec<Tool> {
    vec![
        Tool { name: "add", run: Box::new(add), needs_approval: false },
        Tool { name: "delete_note", run: Box::new(delete_note), needs_approval: true },
    ]
}

/// Only these may run. A tool the model asks for that is not here is refused.
#[must_use]
pub fn allow() -> HashSet<&'static str> {
    HashSet::from(["add"])
}
