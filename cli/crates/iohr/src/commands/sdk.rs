//! `iohr sdk generate`: a surface cut to what the given documents hold, written into a
//! directory of the developer's repository with its `iohr.lock` beside it (RFC 0020).

use std::path::{Path, PathBuf};

use iohr_codegen::{RustTarget, Target as _};
use iohr_openapi::Api;
use serde_json::Value;

use crate::cli::{Lang, SdkGenerate};
use crate::error::Error;
use crate::lock::Lock;
use crate::output::Out;

/// `iohr sdk generate --from NAME=FILE ... --out DIR`.
pub(crate) fn generate(args: &SdkGenerate, out: Out) -> Result<(), Error> {
    let mut docs: Vec<(String, Value)> = Vec::new();
    for spec in &args.from {
        let (profile, file) = spec.split_once('=').ok_or_else(|| {
            Error::Usage(format!(
                "--from takes NAME=FILE, the profile's name and the document it saw; got {spec:?}"
            ))
        })?;
        let text = std::fs::read_to_string(file)
            .map_err(|e| Error::Failed(format!("cannot read {file}: {e}")))?;
        let doc: Value = serde_json::from_str(&text)
            .map_err(|e| Error::Failed(format!("{file} is not a JSON document: {e}")))?;
        docs.push((profile.to_owned(), doc));
    }
    if docs.is_empty() {
        return Err(Error::Usage(
            "give at least one document: --from NAME=FILE (a document saved with `iohr openapi pull`)".into(),
        ));
    }
    let api = Api::from_documents(docs).map_err(|e| Error::Failed(e.to_string()))?;
    write_surface(&api, args, out)
}

/// Renders `api` for the chosen language into `--out`, with the lock beside it.
pub(crate) fn write_surface(api: &Api, args: &SdkGenerate, out: Out) -> Result<(), Error> {
    let files = match args.lang {
        Lang::Rust => RustTarget
            .render(api, &args.rust)
            .map_err(|e| Error::Failed(e.to_string()))?,
    };
    let target = &args.out;
    let occupied = std::fs::read_dir(target).is_ok_and(|mut d| d.next().is_some());
    if !args.force && occupied {
        return Err(Error::Usage(format!(
            "{} is not empty; pass --force to replace what is there with the generated surface",
            target.display()
        )));
    }
    files
        .write(target, true)
        .map_err(|e| Error::Failed(format!("cannot write {}: {e}", target.display())))?;
    let lock_path = lock_path(target);
    let out_name = target
        .file_name()
        .map_or_else(|| ".".into(), |n| n.to_string_lossy().into_owned());
    let lock = Lock::from_api(
        api,
        lang_name(args.lang),
        &out_name,
        env!("CARGO_PKG_VERSION"),
    );
    if !args.rust.in_crate {
        std::fs::write(&lock_path, lock.render())
            .map_err(|e| Error::Failed(format!("cannot write {}: {e}", lock_path.display())))?;
    }
    for note in files.notes() {
        Out::note(&format!("note: {note}"));
    }
    if out.json {
        Out::print_json(&serde_json::json!({
            "out": target,
            "lock": if args.rust.in_crate { None } else { Some(&lock_path) },
            "files": files.len(),
            "profiles": lock.profiles,
            "notes": files.notes(),
        }));
    } else {
        let ops: usize = lock.profiles.values().map(|p| p.operations.len()).sum();
        let profiles: Vec<&str> = lock.profiles.keys().map(String::as_str).collect();
        Out::note(&format!(
            "Wrote {} files to {} for {} ({ops} operation{} across the profiles){}.",
            files.len(),
            target.display(),
            profiles.join(", "),
            if ops == 1 { "" } else { "s" },
            if args.rust.in_crate {
                String::new()
            } else {
                format!("; the lock is {}", lock_path.display())
            }
        ));
    }
    Ok(())
}

/// `iohr.lock` beside the output directory.
pub(crate) fn lock_path(out: &Path) -> PathBuf {
    out.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("iohr.lock"), |p| p.join("iohr.lock"))
}

pub(crate) fn lang_name(lang: Lang) -> &'static str {
    match lang {
        Lang::Rust => "rust",
    }
}
