//! `iohr ext ...` and `iohr <extension> ...`.

use std::ffi::OsString;
use std::path::Path;

use crate::Env;
use crate::cli::{ExtCommand, Global};
use crate::context::Ctx;
use crate::error::Error;
use crate::ext::install::{ExtError, Fetched, Store, Want, fetch};
use crate::ext::lock::{Entry, Lock};
use crate::ext::manifest::{check_name, check_version};
use crate::ext::oci::{DEFAULT_REGISTRY, Digest, OciError, Registry, Source};
use crate::ext::trust::{Policy, TrustedKey};
use crate::output::Out;

impl From<ExtError> for Error {
    fn from(e: ExtError) -> Self {
        match e {
            ExtError::Usage(m) => Self::Usage(m),
            other => Self::Failed(other.to_string()),
        }
    }
}

impl From<OciError> for Error {
    fn from(e: OciError) -> Self {
        Self::from(ExtError::from(e))
    }
}

pub(crate) fn store(g: &Global) -> Result<Store, Error> {
    Ok(Store::new(match &g.data_dir {
        Some(d) => d.join("extensions"),
        None => Store::default_root()?,
    }))
}

/// The registry and trust policy from the config (and `IOHR_EXT_REGISTRY`).
fn setup(g: &Global, env: &Env) -> Result<(Registry, Policy), Error> {
    let ctx = Ctx::load(g)?;
    let raw = env
        .ext_registry
        .clone()
        .or_else(|| ctx.config.ext.registry.clone())
        .unwrap_or_else(|| DEFAULT_REGISTRY.to_owned());
    let source = Source::parse(&raw)?;
    let keys = ctx
        .config
        .ext
        .trusted_keys
        .iter()
        .map(|pem| TrustedKey::from_pem(pem))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::Failed(format!("ext.trusted_keys: {e}")))?;
    let registry = Registry::new(source, env.ext_registry_auth.clone(), g.verbose)?;
    Ok((registry, Policy::inorbit(keys)))
}

pub(crate) async fn run(g: &Global, env: &Env, cmd: ExtCommand, out: Out) -> Result<(), Error> {
    let store = store(g)?;
    match cmd {
        ExtCommand::Install { extension, lock } => {
            let (name, want) = parse_spec(&extension)?;
            let (registry, policy) = setup(g, env)?;
            let want = match want {
                Some(w) => w,
                None => Want::Version(newest(&registry, &name).await?),
            };
            let f = fetch(&registry, &policy, &name, &want).await?;
            let entry = keep(&store, &registry, &f, lock.as_deref(), None)?;
            report(&f, &entry, out);
            Ok(())
        }
        ExtCommand::List => list(&store, out),
        ExtCommand::Upgrade { name, lock } => {
            upgrade(g, env, &store, name, lock.as_deref(), out).await
        }
        ExtCommand::Remove { name, lock } => {
            check_name(&name)?;
            let removed = store.remove(&name)?;
            if let Some(path) = lock.as_deref() {
                let mut l = Lock::load(path)?;
                if l.entries.remove(&name).is_some() {
                    l.save(path)?;
                }
            }
            if !removed {
                return Err(Error::Usage(format!("{name} is not installed")));
            }
            Out::note(&format!("Removed {name}."));
            Ok(())
        }
        ExtCommand::Verify { name } => verify(g, env, &store, name, out),
        ExtCommand::Sync { lock } => sync(g, env, &store, &lock, out).await,
    }
}

/// `agent`, `agent@1.2.3` or `agent@sha256:...`.
fn parse_spec(spec: &str) -> Result<(String, Option<Want>), Error> {
    let (name, at) = match spec.split_once('@') {
        Some((n, v)) => (n, Some(v)),
        None => (spec, None),
    };
    check_name(name)?;
    let want = match at {
        None => None,
        Some(d) if d.starts_with("sha256:") => Some(Want::Digest(Digest::parse(d)?)),
        Some(v) => {
            check_version(v).map_err(|e| Error::Usage(e.to_string()))?;
            Some(Want::Version(v.to_owned()))
        }
    };
    Ok((name.to_owned(), want))
}

/// The newest release tag: the highest version without a pre-release, or the highest
/// pre-release when there is no release yet.
async fn newest(registry: &Registry, name: &str) -> Result<String, Error> {
    let tags = registry
        .tags(&registry.source().repository(name))
        .await
        .map_err(|e| match e {
            OciError::NotFound(_) => Error::Usage(format!(
                "there is no extension {name} in {}",
                registry.source().display()
            )),
            other => other.into(),
        })?;
    let mut versions: Vec<semver::Version> =
        tags.iter().filter_map(|t| check_version(t).ok()).collect();
    versions.sort();
    let pick = versions
        .iter()
        .rev()
        .find(|v| v.pre.is_empty())
        .or_else(|| versions.last())
        .ok_or_else(|| {
            Error::Usage(format!(
                "{name} has no released version in {}",
                registry.source().display()
            ))
        })?;
    Ok(pick.to_string())
}

fn keep(
    store: &Store,
    registry: &Registry,
    f: &Fetched,
    lock: Option<&Path>,
    pinned_signer: Option<&str>,
) -> Result<Entry, Error> {
    if let Some(want) = pinned_signer
        && f.verified.signer.lock_name() != want
    {
        return Err(Error::Failed(format!(
            "{} {} is signed by {}, but the lock pins {want}: not installed",
            f.manifest.name,
            f.manifest.version,
            f.verified.signer.lock_name()
        )));
    }
    let entry = store.install(f, &registry.source().display())?;
    if let Some(path) = lock {
        let mut l = Lock::load(path)?;
        l.entries.insert(entry.name.clone(), entry.clone());
        l.save(path)?;
    }
    Ok(entry)
}

fn report(f: &Fetched, entry: &Entry, out: Out) {
    let m = &f.manifest;
    if out.json {
        Out::print_json(&serde_json::json!({
            "name": m.name, "version": m.version, "digest": entry.digest,
            "signer": f.verified.signer.to_string(), "provenance": f.verified.provenance,
            "scopes": m.scopes,
        }));
        return;
    }
    Out::note(&format!(
        "Installed {} {} ({}).\n  signed by   {}\n  built from  {}\n  may ask for {}\nRun it as `iohr {} ...`.",
        m.name,
        m.version,
        entry.digest,
        f.verified.signer,
        f.verified
            .provenance
            .as_deref()
            .unwrap_or("(no provenance; a configured key signed it)"),
        if m.scopes.is_empty() {
            "no API access".to_owned()
        } else {
            m.scopes.join(", ")
        },
        m.name
    ));
}

fn list(store: &Store, out: Out) -> Result<(), Error> {
    let lock = store.lock()?;
    let mut rows = Vec::new();
    let mut json = Vec::new();
    for name in lock.entries.keys() {
        let Some((entry, record, _)) = store.installed(name)? else {
            continue;
        };
        json.push(serde_json::json!({
            "name": entry.name, "version": entry.version, "digest": entry.digest,
            "signer": record.signer, "provenance": record.provenance,
            "scopes": record.manifest.scopes, "registry": record.registry,
            "installed_at": record.installed_at,
        }));
        rows.push(vec![
            entry.name.clone(),
            entry.version.clone(),
            Digest::parse(&entry.digest)
                .map_or_else(|_| entry.digest.clone(), |d| d.short().to_owned()),
            record.manifest.scopes.join(" "),
            entry.signer.clone(),
        ]);
    }
    if out.json {
        Out::print_json(&json);
    } else if rows.is_empty() {
        Out::note("No extensions installed. `iohr ext install agent` installs the InOrbit agent.");
    } else {
        Out::table(&["NAME", "VERSION", "DIGEST", "SCOPES", "SIGNER"], &rows);
    }
    Ok(())
}

async fn upgrade(
    g: &Global,
    env: &Env,
    store: &Store,
    name: Option<String>,
    lock: Option<&Path>,
    out: Out,
) -> Result<(), Error> {
    let installed = store.lock()?;
    let names: Vec<String> = match name {
        Some(n) => {
            if !installed.entries.contains_key(&n) {
                return Err(Error::Usage(format!(
                    "{n} is not installed; `iohr ext install {n}` installs it"
                )));
            }
            vec![n]
        }
        None => installed.entries.keys().cloned().collect(),
    };
    if names.is_empty() {
        Out::note("No extensions installed.");
        return Ok(());
    }
    let (registry, policy) = setup(g, env)?;
    for n in names {
        let current = &installed.entries[&n];
        let latest = newest(&registry, &n).await?;
        let newer = match (check_version(&latest), check_version(&current.version)) {
            (Ok(l), Ok(c)) => l > c,
            _ => false,
        };
        if !newer {
            Out::note(&format!("{n} {} is the newest.", current.version));
            continue;
        }
        let before = store
            .installed(&n)?
            .map(|(_, r, _)| r.manifest.scopes)
            .unwrap_or_default();
        let f = fetch(&registry, &policy, &n, &Want::Version(latest)).await?;
        let added: Vec<&String> = f
            .manifest
            .scopes
            .iter()
            .filter(|s| !before.contains(s))
            .collect();
        let entry = keep(store, &registry, &f, lock, None)?;
        report(&f, &entry, out);
        if !added.is_empty() {
            Out::note(&format!(
                "  new scopes  {} (it may now ask for these)",
                added
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    Ok(())
}

fn verify(
    g: &Global,
    env: &Env,
    store: &Store,
    name: Option<String>,
    out: Out,
) -> Result<(), Error> {
    let (_, policy) = setup(g, env)?;
    let names: Vec<String> = match name {
        Some(n) => vec![n],
        None => store.lock()?.entries.keys().cloned().collect(),
    };
    let mut failed = Vec::new();
    let mut rows = Vec::new();
    for n in &names {
        match store.verify(n, &policy) {
            Ok((entry, record)) => {
                rows.push(vec![n.clone(), entry.version, "ok".into(), record.signer]);
            }
            Err(ExtError::Usage(m)) => return Err(Error::Usage(m)),
            Err(e) => {
                rows.push(vec![
                    n.clone(),
                    String::new(),
                    "FAILED".into(),
                    e.to_string(),
                ]);
                failed.push(n.clone());
            }
        }
    }
    if out.json {
        let v: Vec<_> = rows
            .iter()
            .map(|r| serde_json::json!({"name": r[0], "version": r[1], "ok": r[2] == "ok", "detail": r[3]}))
            .collect();
        Out::print_json(&v);
    } else if rows.is_empty() {
        Out::note("No extensions installed.");
    } else {
        Out::table(&["NAME", "VERSION", "CHECK", "SIGNER OR FAILURE"], &rows);
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(Error::Failed(format!(
            "{} failed verification; do not run it. Remove it with `iohr ext remove`, then install it again.",
            failed.join(", ")
        )))
    }
}

async fn sync(g: &Global, env: &Env, store: &Store, path: &Path, out: Out) -> Result<(), Error> {
    if !path.exists() {
        return Err(Error::Usage(format!(
            "{} does not exist; `iohr ext install NAME --lock {}` writes one",
            path.display(),
            path.display()
        )));
    }
    let wanted = Lock::load(path)?;
    let installed = store.lock()?;
    let (registry, policy) = setup(g, env)?;
    let mut done = Vec::new();
    for (name, e) in &wanted.entries {
        if installed.entries.get(name) == Some(e) && store.verify(name, &policy).is_ok() {
            done.push(format!("{name} {} already installed", e.version));
            continue;
        }
        let digest = Digest::parse(&e.digest)?;
        let f = fetch(&registry, &policy, name, &Want::Digest(digest)).await?;
        if f.manifest.version != e.version {
            return Err(Error::Failed(format!(
                "{} pins {name} {} at {}, but that digest holds version {}",
                path.display(),
                e.version,
                e.digest,
                f.manifest.version
            )));
        }
        keep(store, &registry, &f, None, Some(&e.signer))?;
        done.push(format!("{name} {} installed", e.version));
    }
    let extra: Vec<&String> = installed
        .entries
        .keys()
        .filter(|n| !wanted.entries.contains_key(*n))
        .collect();
    if out.json {
        Out::print_json(&serde_json::json!({ "synced": done, "not_in_lock": extra }));
        return Ok(());
    }
    for d in &done {
        Out::note(d);
    }
    if !extra.is_empty() {
        Out::note(&format!(
            "Also installed here, not in {}: {} (left as they are).",
            path.display(),
            extra
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(())
}

/// `iohr <extension> ...`.
pub(crate) async fn external(g: &Global, env: &Env, argv: Vec<OsString>) -> Result<(), Error> {
    let mut argv = argv.into_iter();
    let name = argv
        .next()
        .and_then(|n| n.into_string().ok())
        .unwrap_or_default();
    if check_name(&name).is_err() {
        return Err(Error::Usage(format!(
            "`{}` is not an iohr command; `iohr --help` lists them",
            crate::ext::manifest::printable(&name)
        )));
    }
    let rest: Vec<OsString> = argv.collect();
    let code = crate::ext::run::run(g, env, &store(g)?, &name, &rest).await?;
    if code == 0 {
        Ok(())
    } else {
        Err(Error::Child(code))
    }
}

#[cfg(test)]
mod tests {
    use super::parse_spec;
    use crate::ext::install::Want;

    #[test]
    fn specs() {
        assert_eq!(
            parse_spec("agent").ok().map(|(n, w)| (n, w.is_none())),
            Some(("agent".into(), true))
        );
        assert!(
            matches!(parse_spec("agent@1.2.3"), Ok((_, Some(Want::Version(v)))) if v == "1.2.3")
        );
        let d = format!("agent@sha256:{}", "a".repeat(64));
        assert!(matches!(parse_spec(&d), Ok((_, Some(Want::Digest(_))))));
        for bad in ["", "Agent", "agent@1", "agent@sha256:zz", "ext@1.0.0"] {
            assert!(parse_spec(bad).is_err(), "{bad}");
        }
    }
}
