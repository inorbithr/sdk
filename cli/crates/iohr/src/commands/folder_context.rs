//! `iohr context`: the `IO.md` files `?` and `??` would load from a folder, in the order the
//! model gets them. The same loader as the prompt ([`crate::iomd::load`]), so what this
//! prints is what is sent.

use crate::cli::{ContextArgs, Global};
use crate::error::Error;
use crate::iomd::{self, Kind};
use crate::output::Out;
use crate::ui::Paint;
use std::fmt::Write as _;

pub(crate) fn run(g: &Global, args: &ContextArgs, out: Out) -> Result<(), Error> {
    let dir = match &args.dir {
        Some(d) => d.clone(),
        None => std::env::current_dir()
            .map_err(|e| Error::Usage(format!("cannot read the current folder: {e}")))?,
    };
    if !dir.is_dir() {
        return Err(Error::Usage(format!("{} is not a folder", dir.display())));
    }
    let user = iomd::user_dir(g);
    let chain = iomd::load(&dir, user.as_deref());
    if out.json {
        let mut v = chain.json();
        if args.full {
            v["text"] = serde_json::Value::String(chain.prompt());
        }
        Out::print_json(&v);
        return Ok(());
    }
    let p = Paint::stdout(false);
    let mut text = String::new();
    let root = chain.root.as_ref().map_or_else(
        || "none (not in a project: only this folder's own file is read)".to_owned(),
        |r| r.display().to_string(),
    );
    let _ = writeln!(text, "{}  {root}", p.head("project root"));
    if let Some(u) = user
        .as_ref()
        .filter(|_| chain.docs.iter().all(|d| d.kind != Kind::User))
    {
        let _ = writeln!(text, "{}  none in {}", p.head("user file"), u.display());
    }
    if chain.is_empty() {
        text.push_str("\nNo IO.md or io.md found. `iohr project init` writes a starter one.\n");
    } else {
        text.push('\n');
        let rows: Vec<Vec<String>> = chain
            .docs
            .iter()
            .map(|d| {
                vec![
                    d.label.clone(),
                    format!("{}", d.bytes),
                    format!("{}", d.text.len()),
                    if d.cut {
                        "cut".to_owned()
                    } else {
                        String::new()
                    },
                    d.path.display().to_string(),
                ]
            })
            .collect();
        text.push_str(&crate::output::table_text(
            &["FILE", "BYTES", "SENT", "NOTE", "PATH"],
            &rows,
        ));
        text.push_str("\nFarthest first; the nearest wins.\n");
    }
    for s in &chain.skipped {
        let _ = writeln!(
            text,
            "{}  {} sits beside an IO.md and is not read",
            p.bad("warning"),
            s.display()
        );
    }
    for s in &chain.dropped {
        let _ = writeln!(
            text,
            "{}  {} is left out: the files nearer to you fill the {} KiB limit",
            p.bad("warning"),
            s.display(),
            iomd::TOTAL_MAX / 1024
        );
    }
    if chain.cuts() > 0 {
        let _ = writeln!(
            text,
            "note  a file is cut at {} KiB, and all together at {} KiB",
            iomd::FILE_MAX / 1024,
            iomd::TOTAL_MAX / 1024
        );
    }
    if args.full {
        text.push_str("\n--- sent to the model, inside <io-md> ---\n");
        text.push_str(&chain.prompt());
    }
    Out::raw(text.as_bytes());
    Ok(())
}
