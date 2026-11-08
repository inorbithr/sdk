//! `iohr handoff show|compact|clear`: the note an agent run leaves in the project when it stops
//! early (`.inorbit/handoff.md`, see [`crate::handoff`]). Local files only: no sign-in, no
//! network. The note's text goes to stdout only for `show`, which the person asked for.

use serde_json::json;

use crate::cli::HandoffCommand;
use crate::error::Error;
use crate::handoff;
use crate::output::Out;

pub(crate) fn run(cmd: HandoffCommand, out: Out) -> Result<(), Error> {
    let root = handoff::root_here();
    let have = root.as_deref().and_then(handoff::runs);
    match cmd {
        HandoffCommand::Show => {
            let Some(root) = root.filter(|_| have.is_some()) else {
                return none(out);
            };
            let text = std::fs::read_to_string(root.join(handoff::FILE))
                .map_err(|e| Error::Failed(format!("cannot read the note: {}", e.kind())))?;
            if out.json {
                Out::print_json(&json!({
                    "path": handoff::FILE,
                    "runs": have,
                    "text": text,
                }));
            } else {
                Out::raw(text.as_bytes());
            }
            Ok(())
        }
        HandoffCommand::Compact => {
            let Some(root) = root.filter(|_| have.is_some()) else {
                return none(out);
            };
            let n = handoff::compact(&root, &super::shell::running_bin()).map_err(Error::Failed)?;
            if out.json {
                Out::print_json(&json!({ "path": handoff::FILE, "folded": n }));
            } else {
                Out::note(&format!(
                    "iohr handoff: folded {n} earlier runs in {}",
                    handoff::FILE
                ));
            }
            Ok(())
        }
        HandoffCommand::Clear => {
            let Some(root) = root.filter(|_| have.is_some()) else {
                return none(out);
            };
            handoff::clear(&root);
            if out.json {
                Out::print_json(&json!({ "path": handoff::FILE, "removed": true }));
            } else {
                Out::note(&format!("iohr handoff: removed {}", handoff::FILE));
            }
            Ok(())
        }
    }
}

fn none(out: Out) -> Result<(), Error> {
    if out.json {
        Out::print_json(&json!({ "path": handoff::FILE, "runs": 0 }));
    } else {
        Out::note(&format!("iohr handoff: no {} in this project", handoff::FILE));
    }
    Ok(())
}
