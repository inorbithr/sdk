//! `iohr ext ...` and `iohr <extension> ...`.

use std::ffi::OsString;
use std::path::Path;

use crate::Env;
use crate::cli::{ExtCommand, Global};
use crate::context::Ctx;
use crate::error::Error;
use crate::ext::install::{ExtError, Fetched, Store, Want, fetch};
use crate::ext::lock::{Entry, Lock};
use crate::ext::manifest::{check_name, check_version, privilege};
use crate::ext::oci::{DEFAULT_REGISTRY, Digest, OciError, Registry, Source};
use crate::ext::trust::{Policy, TrustedKey};
use crate::output::Out;
use crate::prompt;

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
        ExtCommand::Install {
            extension,
            lock,
            yes,
        } => {
            let (name, want) = parse_spec(&extension)?;
            let (registry, policy) = setup(g, env)?;
            let want = match want {
                Some(w) => w,
                None => Want::Version(newest(&registry, &name).await?),
            };
            let f = fetch(&registry, &policy, &name, &want).await?;
            let declared: Vec<&str> = f.manifest.privileges.iter().map(String::as_str).collect();
            confirm_privileges(
                &format!("{} {}", f.manifest.name, f.manifest.version),
                &declared,
                false,
                yes,
                terminal().as_mut().map(|a| a as &mut Ask),
            )?;
            let entry = keep(&store, &registry, &f, lock.as_deref(), None)?;
            report(&f, &entry, out);
            Ok(())
        }
        ExtCommand::List => list(&store, out),
        ExtCommand::Upgrade { name, lock, yes } => {
            upgrade(g, env, &store, name, lock.as_deref(), yes, out).await
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

/// Asks the person a question and returns the answer.
type Ask<'a> = dyn FnMut(&str) -> Result<String, Error> + 'a;

/// The person at the keyboard, when there is one to ask: stdin and stderr are terminals.
fn terminal() -> Option<impl FnMut(&str) -> Result<String, Error>> {
    (prompt::interactive() && std::io::IsTerminal::is_terminal(&std::io::stdin()))
        .then_some(prompt::line)
}

/// The privileges in plain words, one per line, for a note.
fn privilege_lines(privileges: &[&str]) -> String {
    let w = privileges.iter().map(|p| p.len()).max().unwrap_or(0);
    privileges
        .iter()
        .map(|p| {
            format!(
                "  {p:w$}  {}",
                privilege(p).unwrap_or("not a known capability")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The confirmation SR-32 asks for before an extension that declares privileges (or,
/// on upgrade, new ones) is installed. The privileges are shown in plain words on
/// stderr; `--yes` confirms them, a person at a terminal confirms by typing `yes`, and
/// without either (`ask` is `None`: no terminal) nothing is installed.
fn confirm_privileges(
    subject: &str,
    privileges: &[&str],
    new: bool,
    yes: bool,
    ask: Option<&mut Ask<'_>>,
) -> Result<(), Error> {
    if privileges.is_empty() {
        return Ok(());
    }
    Out::note(&format!(
        "{subject} declares {}privileges: its system service holds these Linux capabilities.\n\
         {}\n\
         iohr runs as you and grants none of them; the service gets them from its own \
         package or unit.",
        if new { "new " } else { "" },
        privilege_lines(privileges),
    ));
    if yes {
        return Ok(());
    }
    let Some(ask) = ask else {
        return Err(Error::Usage(format!(
            "{subject} declares {}privileges ({}): confirm them at a terminal, or pass --yes; \
             nothing was installed",
            if new { "new " } else { "" },
            privileges.join(", ")
        )));
    };
    if ask("Type yes to confirm these privileges and install it")? == "yes" {
        Ok(())
    } else {
        Err(Error::Usage(
            "the privileges were not confirmed; nothing was installed".into(),
        ))
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
            "scopes": m.scopes, "privileges": m.privileges,
        }));
        return;
    }
    Out::note(&format!(
        "Installed {} {} ({}).\n  signed by   {}\n  built from  {}\n  may ask for {}\n  privileges  {}\nRun it as `iohr {} ...`.",
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
        if m.privileges.is_empty() {
            "none".to_owned()
        } else {
            format!(
                "{} (held by its system service, not by iohr)",
                m.privileges.join(", ")
            )
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
            "scopes": record.manifest.scopes, "privileges": record.manifest.privileges,
            "registry": record.registry,
            "installed_at": record.installed_at,
        }));
        rows.push(vec![
            entry.name.clone(),
            entry.version.clone(),
            Digest::parse(&entry.digest)
                .map_or_else(|_| entry.digest.clone(), |d| d.short().to_owned()),
            record.manifest.scopes.join(" "),
            record.manifest.privileges.join(" "),
            entry.signer.clone(),
        ]);
    }
    if out.json {
        Out::print_json(&json);
    } else if rows.is_empty() {
        Out::note("No extensions installed. `iohr ext install agent` installs the InOrbit agent.");
    } else {
        Out::table(
            &[
                "NAME",
                "VERSION",
                "DIGEST",
                "SCOPES",
                "PRIVILEGES",
                "SIGNER",
            ],
            &rows,
        );
    }
    Ok(())
}

async fn upgrade(
    g: &Global,
    env: &Env,
    store: &Store,
    name: Option<String>,
    lock: Option<&Path>,
    yes: bool,
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
        let more = current.unconfirmed(&f.manifest.privileges);
        confirm_privileges(
            &format!("{n} {}", f.manifest.version),
            &more,
            true,
            yes,
            terminal().as_mut().map(|a| a as &mut Ask),
        )?;
        let entry = keep(store, &registry, &f, lock, None)?;
        report(&f, &entry, out);
        if !more.is_empty() {
            Out::note(&format!(
                "  new privileges {} (its system service holds these)",
                more.join(", ")
            ));
        }
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
                rows.push(vec![
                    n.clone(),
                    entry.version,
                    "ok".into(),
                    record.manifest.privileges.join(" "),
                    record.signer,
                ]);
            }
            Err(ExtError::Usage(m)) => return Err(Error::Usage(m)),
            Err(e) => {
                rows.push(vec![
                    n.clone(),
                    String::new(),
                    "FAILED".into(),
                    String::new(),
                    e.to_string(),
                ]);
                failed.push(n.clone());
            }
        }
    }
    if out.json {
        let v: Vec<_> = rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "name": r[0], "version": r[1], "ok": r[2] == "ok",
                    "privileges": r[3].split_whitespace().collect::<Vec<_>>(), "detail": r[4],
                })
            })
            .collect();
        Out::print_json(&v);
    } else if rows.is_empty() {
        Out::note("No extensions installed.");
    } else {
        Out::table(
            &[
                "NAME",
                "VERSION",
                "CHECK",
                "PRIVILEGES",
                "SIGNER OR FAILURE",
            ],
            &rows,
        );
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
        let grown = e.unconfirmed(&f.manifest.privileges);
        if !grown.is_empty() {
            return Err(Error::Failed(format!(
                "{name} {} at {} declares privileges {} does not record ({}): not installed. \
                 Review them, then `iohr ext install {name}@{} --lock {}` confirms and pins them.",
                e.version,
                e.digest,
                path.display(),
                grown.join(", "),
                e.version,
                path.display()
            )));
        }
        keep(store, &registry, &f, None, Some(&e.signer))?;
        if f.manifest.privileges.is_empty() {
            done.push(format!("{name} {} installed", e.version));
        } else {
            done.push(format!(
                "{name} {} installed; its system service holds {} (confirmed in {})",
                e.version,
                f.manifest.privileges.join(", "),
                path.display()
            ));
        }
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
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::{Ask, confirm_privileges, parse_spec, privilege_lines};
    use crate::error::Error;
    use crate::ext::install::Want;

    const CAPS: &[&str] = &["CAP_BPF", "CAP_PERFMON", "CAP_NET_ADMIN"];

    /// An answer at the terminal, and whether the question was asked.
    fn answering<'a>(
        answer: &'static str,
        asked: &'a std::cell::Cell<bool>,
    ) -> impl FnMut(&str) -> Result<String, Error> + use<'a> {
        move |q: &str| {
            asked.set(true);
            assert!(q.contains("yes"), "{q}");
            Ok(answer.to_owned())
        }
    }

    #[test]
    fn no_privileges_need_no_confirmation() {
        assert!(confirm_privileges("agent 1.0.0", &[], false, false, None).is_ok());
    }

    #[test]
    fn a_person_at_a_terminal_confirms_by_typing_yes() {
        let asked = std::cell::Cell::new(false);
        let mut ask = answering("yes", &asked);
        assert!(
            confirm_privileges(
                "capture 0.1.0",
                CAPS,
                false,
                false,
                Some(&mut ask as &mut Ask)
            )
            .is_ok()
        );
        assert!(asked.get());
        for no in ["", "no", "y", "YES please"] {
            let asked = std::cell::Cell::new(false);
            let mut ask = answering(no, &asked);
            let e = confirm_privileges(
                "capture 0.1.0",
                CAPS,
                false,
                false,
                Some(&mut ask as &mut Ask),
            )
            .unwrap_err();
            assert!(asked.get());
            assert!(
                matches!(&e, Error::Usage(m) if m.contains("not confirmed")),
                "{e}"
            );
        }
    }

    #[test]
    fn yes_confirms_without_asking() {
        let asked = std::cell::Cell::new(false);
        let mut ask = answering("no", &asked);
        assert!(
            confirm_privileges(
                "capture 0.1.0",
                CAPS,
                false,
                true,
                Some(&mut ask as &mut Ask)
            )
            .is_ok()
        );
        assert!(!asked.get(), "--yes must not ask");
        assert!(confirm_privileges("capture 0.1.0", CAPS, true, true, None).is_ok());
    }

    #[test]
    fn without_a_terminal_or_yes_nothing_is_installed() {
        let e = confirm_privileges("capture 0.1.0", CAPS, false, false, None).unwrap_err();
        let Error::Usage(m) = &e else { panic!("{e}") };
        assert!(
            m.contains("--yes") && m.contains("CAP_BPF, CAP_PERFMON, CAP_NET_ADMIN"),
            "{m}"
        );
        let e =
            confirm_privileges("capture 0.2.0", &["CAP_NET_RAW"], true, false, None).unwrap_err();
        assert!(
            e.to_string().contains("new privileges (CAP_NET_RAW)"),
            "{e}"
        );
    }

    #[test]
    fn privileges_are_shown_in_plain_words() {
        let text = privilege_lines(CAPS);
        assert_eq!(
            text,
            "  CAP_BPF        load eBPF programs into the kernel\n\
             \x20 CAP_PERFMON    observe performance and kernel state (perf events, tracing)\n\
             \x20 CAP_NET_ADMIN  configure the network: interfaces, routes, firewall, traffic control"
        );
    }

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
