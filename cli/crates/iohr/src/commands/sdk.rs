//! `iohr sdk generate` and `iohr sdk check` (RFC 0020): a surface cut to what each
//! profile's credential may call, written into the developer's repository with its
//! `iohr.lock` beside it; and the check that says when the API's cut has moved.

use std::path::{Path, PathBuf};

use iohr_auth::{Kind, ProfileName};
use iohr_codegen::{Files, Language};
use iohr_openapi::Api;
use serde_json::Value;

use crate::Env;
use crate::cli::{Global, SdkCheck, SdkGenerate};
use crate::context::{Ctx, Session, session_for};
use crate::error::Error;
use crate::lock::Lock;
use crate::output::Out;

/// `iohr sdk generate`: from profiles (`--profile`), from files (`--from`), or both.
pub(crate) async fn generate(
    g: &Global,
    env: &Env,
    args: &SdkGenerate,
    out: Out,
) -> Result<(), Error> {
    let mut docs = from_files(&args.from)?;
    if !args.profiles.is_empty() {
        let ctx = Ctx::load(g)?;
        for name in &args.profiles {
            docs.push((name.to_string(), fetch_cut(g, env, &ctx, name).await?));
        }
    }
    if docs.is_empty() {
        return Err(Error::Usage(
            "give at least one profile (--for NAME) or document (--from NAME=FILE)".into(),
        ));
    }
    let api = Api::from_documents(docs).map_err(|e| Error::Failed(e.to_string()))?;
    write_surface(&api, args, out)
}

/// `iohr sdk check`: every profile in the lock is fetched again and compared.
pub(crate) async fn check(g: &Global, env: &Env, args: &SdkCheck, out: Out) -> Result<(), Error> {
    let lock = Lock::read(&args.lock)?;
    let ctx = Ctx::load(g)?;
    let mut docs = Vec::new();
    for name in lock.profiles.keys() {
        let profile: ProfileName = name.parse().map_err(|_| {
            Error::Failed(format!(
                "{}: {name:?} is not a profile name",
                args.lock.display()
            ))
        })?;
        docs.push((name.clone(), fetch_cut(g, env, &ctx, &profile).await?));
    }
    let api = Api::from_documents(docs).map_err(|e| Error::Failed(e.to_string()))?;
    let drift = lock.drift(&api);
    let mut files_changed = Vec::new();
    if args.files {
        let out_dir = args
            .lock
            .parent()
            .map_or_else(|| PathBuf::from(&lock.out), |p| p.join(&lock.out));
        let lang: Language = lock
            .lang
            .parse()
            .map_err(|e: iohr_codegen::RenderError| Error::Failed(e.to_string()))?;
        let files = render(&api, lang, &lock.options)?;
        files_changed = files
            .diff(&out_dir)
            .map_err(|e| Error::Failed(format!("cannot read {}: {e}", out_dir.display())))?;
    }
    if out.json {
        Out::print_json(&serde_json::json!({
            "lock": args.lock,
            "drift": drift,
            "files_changed": files_changed,
            "ok": drift.is_empty() && files_changed.is_empty(),
        }));
    } else {
        for d in &drift {
            let mut lines = vec![format!(
                "profile {}: the cut moved ({} -> {})",
                d.profile,
                short(&d.was),
                short(&d.now)
            )];
            lines.extend(d.added.iter().map(|l| format!("  + {l}")));
            lines.extend(d.removed.iter().map(|l| format!("  - {l}")));
            Out::raw(format!("{}\n", lines.join("\n")).as_bytes());
        }
        for f in &files_changed {
            Out::raw(format!("{f}\n").as_bytes());
        }
    }
    if !drift.is_empty() || !files_changed.is_empty() {
        let files = if files_changed.is_empty() {
            String::new()
        } else {
            format!(
                ", {} file{} differ",
                files_changed.len(),
                if files_changed.len() == 1 { "" } else { "s" }
            )
        };
        return Err(Error::Drift(format!(
            "{} profile{} moved{files}; run `iohr sdk generate` again and commit the result",
            drift.len(),
            if drift.len() == 1 { "" } else { "s" },
        )));
    }
    if !out.json {
        Out::note(&format!(
            "{}: every profile's cut is the one the surface was generated from.",
            args.lock.display()
        ));
    }
    Ok(())
}

/// The document a profile sees now: `GET /v1/openapi.json`, with `?account=` for a
/// person, and the stamp checked to name that account, so a gateway that ignored the
/// parameter can never hand back the personal plan as a team's.
async fn fetch_cut(g: &Global, env: &Env, ctx: &Ctx, name: &ProfileName) -> Result<Value, Error> {
    let s: Session = session_for(g, env, ctx, Some(name)).await?;
    let asked = (s.kind == Kind::Person && !s.account.is_empty()).then(|| s.account.clone());
    let query: Vec<(&str, &str)> = asked.iter().map(|a| ("account", a.as_str())).collect();
    let resp = s
        .api
        .send(inorbithr::Method::Get, "/v1/openapi.json", &query, None)
        .await
        .map_err(|e| match e.status() {
            Some(403) => Error::with_hint(
                e,
                "The profile's account is not one its credential may act for.",
            ),
            _ => e.into(),
        })?;
    let doc: Value = resp.json()?;
    if let Some(account) = &asked {
        let stamped = doc
            .pointer("/info/x-iohr-cut/account")
            .and_then(Value::as_str);
        if stamped != Some(account.as_str()) {
            return Err(Error::Failed(format!(
                "profile {name}: the API answered a document for {} where {account} was asked for; the API does not cut by account yet, or the profile's account is wrong (`iohr profile account {name} <id>`)",
                stamped.unwrap_or("no account")
            )));
        }
    }
    Ok(doc)
}

fn from_files(specs: &[String]) -> Result<Vec<(String, Value)>, Error> {
    let mut docs = Vec::new();
    for spec in specs {
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
    Ok(docs)
}

fn render(api: &Api, lang: Language, options: &iohr_codegen::Options) -> Result<Files, Error> {
    iohr_codegen::render(lang, api, options).map_err(|e| Error::Failed(e.to_string()))
}

/// Renders `api` into `--out`, with the lock beside it.
pub(crate) fn write_surface(api: &Api, args: &SdkGenerate, out: Out) -> Result<(), Error> {
    let lang = args.lang.language();
    let mut options = args.options.clone();
    if lang == Language::Go && options.package.is_none() && !options.in_package {
        // Go needs the import path of the output; the enclosing go.mod says it.
        options.package = iohr_codegen::go::package_for(&args.out);
    }
    let files = render(api, lang, &options)?;
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
    let mut lock = Lock::from_api(api, lang.as_str(), &out_name, env!("CARGO_PKG_VERSION"));
    lock.options = iohr_codegen::Options {
        in_package: false,
        ..options
    };
    let in_package = args.options.in_package;
    if !in_package {
        std::fs::write(&lock_path, lock.render())
            .map_err(|e| Error::Failed(format!("cannot write {}: {e}", lock_path.display())))?;
    }
    for note in files.notes() {
        Out::note(&format!("note: {note}"));
    }
    if out.json {
        Out::print_json(&serde_json::json!({
            "out": target,
            "lock": if in_package { None } else { Some(&lock_path) },
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
            if in_package {
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

fn short(hash: &str) -> &str {
    hash.get(..19).unwrap_or(hash)
}
