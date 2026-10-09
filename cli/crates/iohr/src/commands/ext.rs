//! `iohr ext ...` and `iohr <extension> ...`.

use std::ffi::OsString;
use std::path::Path;

use crate::Env;
use crate::cli::{ExtCommand, Global};
use crate::commands::ext_catalogue;
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
            no_service,
            interfaces,
            all_interfaces,
        } => {
            let (registry, policy) = setup(g, env)?;
            let f = if extension.contains('/') {
                from_catalogue(g, env, &store, &registry, &policy, &extension).await?
            } else {
                let (name, want) = parse_spec(&extension)?;
                let want = match want {
                    Some(w) => w,
                    None => Want::Version(newest(&registry, &name).await?),
                };
                fetch(&registry, &policy, &name, &want).await?
            };
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
            if f.manifest.service && !no_service {
                offer_service(
                    &store,
                    &f.manifest.name,
                    &Picks::from(interfaces, all_interfaces),
                    yes,
                    terminal().as_mut().map(|a| a as &mut Ask),
                )?;
            }
            Ok(())
        }
        ExtCommand::Service {
            name,
            interfaces,
            all_interfaces,
        } => {
            check_name(&name)?;
            let picks = Picks::from(interfaces, all_interfaces);
            let picks = if picks == Picks::Default {
                ask_interfaces(&store, &name, terminal().as_mut().map(|a| a as &mut Ask))?
            } else {
                picks
            };
            service(&store, &name, &picks)
        }
        ExtCommand::Search {
            query,
            kind,
            all,
            page_size,
        } => {
            ext_catalogue::search(
                g,
                env,
                query.as_deref(),
                kind.as_deref(),
                all,
                page_size,
                out,
            )
            .await
        }
        ExtCommand::Show { extension } => ext_catalogue::show(g, env, &extension, out).await,
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

/// `PUBLISHER/NAME[@VERSION|@sha256:DIGEST]`: the catalogue names the version, its
/// digest and its signer; the artifact is fetched by that digest from the registry,
/// verified exactly as the registry form is, and must then match what the catalogue
/// listed (`ext_catalogue::matches`).
async fn from_catalogue(
    g: &Global,
    env: &Env,
    store: &Store,
    registry: &Registry,
    policy: &Policy,
    spec: &str,
) -> Result<Fetched, Error> {
    let (listing, at) = match spec.split_once('@') {
        Some((l, a)) => (l, Some(a)),
        None => (spec, None),
    };
    let (publisher, name) = ext_catalogue::parse_listing(listing)?;
    let at = ext_catalogue::parse_at(at)?;
    if publisher != ext_catalogue::FIRST_PARTY && registry.source().display() == DEFAULT_REGISTRY {
        return Err(Error::Usage(format!(
            "{publisher}/{name} is not InOrbit's, and {DEFAULT_REGISTRY} holds only InOrbit's \
             extensions: set ext.registry to the registry {publisher} publishes it in"
        )));
    }
    let r = ext_catalogue::resolve(g, env, &publisher, &name, &at).await?;
    Out::note(&format!(
        "{publisher}/{name}: the catalogue lists {} at {}.",
        r.version, r.digest
    ));
    let f = fetch(registry, policy, &name, &Want::Digest(r.digest.clone())).await?;
    ext_catalogue::matches(&r, &f)?;
    // The store keys extensions by name alone: one signed by someone else never
    // replaces an installed one silently (another publisher's `agent`, say).
    if let Some(installed) = store.lock()?.entries.get(&name) {
        let incoming = f.verified.signer.lock_name();
        if installed.signer != incoming {
            return Err(Error::Failed(format!(
                "{name} is installed from {}, and {publisher}/{name} is signed by {incoming}: \
                 not installed. `iohr ext remove {name}` first to replace it.",
                installed.signer
            )));
        }
    }
    Ok(f)
}

/// Asks the person a question and returns the answer.
type Ask<'a> = dyn FnMut(&str) -> Result<String, Error> + 'a;

/// The person at the keyboard, when there is one to ask: stdin and stderr are terminals.
fn terminal() -> Option<impl FnMut(&str) -> Result<String, Error>> {
    (prompt::interactive() && std::io::IsTerminal::is_terminal(&std::io::stdin()))
        .then_some(prompt::line)
}

/// Which interfaces the system service attaches to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Picks {
    /// The program's own default (the default route's interface).
    Default,
    /// These names.
    Named(Vec<String>),
    /// The program's `--all`, chosen again at each start.
    All,
}

impl Picks {
    fn from(named: Vec<String>, all: bool) -> Self {
        if all {
            Self::All
        } else if named.is_empty() {
            Self::Default
        } else {
            Self::Named(named)
        }
    }
}

/// The command that sets up an installed extension's system service.
fn service_command(program: &Path, user: &str, picks: &Picks) -> Vec<OsString> {
    let mut c: Vec<OsString> = vec![
        program.as_os_str().to_owned(),
        "service".into(),
        "install".into(),
        "--agent-user".into(),
        user.into(),
    ];
    match picks {
        Picks::Default => {}
        Picks::All => c.push("--all".into()),
        Picks::Named(names) => {
            for n in names {
                c.push("--interface".into());
                c.push(n.into());
            }
        }
    }
    c
}

fn shown(c: &[OsString]) -> String {
    let words: Vec<String> = c.iter().map(|w| w.to_string_lossy().into_owned()).collect();
    format!("sudo {}", words.join(" "))
}

fn current_user() -> Result<String, Error> {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .map_err(|_| Error::Usage("neither USER nor LOGNAME is set: say who runs the agent".into()))
}

/// The installed, unchanged program of an extension whose privileges were confirmed.
fn service_program(store: &Store, name: &str) -> Result<std::path::PathBuf, Error> {
    let Some((entry, record, dir)) = store.installed(name)? else {
        return Err(Error::Usage(format!(
            "{name} is not installed: iohr ext install {name}"
        )));
    };
    if !record.manifest.service {
        return Err(Error::Usage(format!(
            "{name} has no system service of its own"
        )));
    }
    crate::ext::install::check_confirmed(&entry, &record)?;
    Ok(crate::ext::install::check_program(&record, &dir)?)
}

/// What the program's `interfaces --json` says: each interface, whether `--all` picks it
/// and why, and the suggestion. Read-only, run as the person (no sudo).
#[derive(Debug, Default, serde::Deserialize)]
struct Detected {
    #[serde(default)]
    interfaces: Vec<DetectedInterface>,
    #[serde(default)]
    suggested: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct DetectedInterface {
    name: String,
    picked: bool,
    why: String,
}

fn detect(program: &Path) -> Option<Detected> {
    let out = std::process::Command::new(program)
        .args(["interfaces", "--json"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| serde_json::from_slice(&out.stdout).ok())
        .flatten()
}

/// The person's answer to the interface question: `yes` takes the suggestion, `all`
/// the program's --all, names (spaces or commas) those names; anything else nothing.
fn parse_choice(answer: &str, suggested: &[String]) -> Option<Picks> {
    let a = answer.trim();
    match a {
        "" | "no" | "n" => None,
        "yes" | "y" => (!suggested.is_empty()).then(|| Picks::Named(suggested.to_vec())),
        "all" => Some(Picks::All),
        _ => {
            let names: Vec<String> = a
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            (!names.is_empty()).then_some(Picks::Named(names))
        }
    }
}

/// At a terminal, lists the interfaces the program detects with its suggestion and asks;
/// without one, the program's default.
fn ask_interfaces(store: &Store, name: &str, ask: Option<&mut Ask<'_>>) -> Result<Picks, Error> {
    let Some(ask) = ask else {
        return Ok(Picks::Default);
    };
    let program = service_program(store, name)?;
    let Some(d) = detect(&program) else {
        return Ok(Picks::Default);
    };
    let w = d.interfaces.iter().map(|i| i.name.len()).max().unwrap_or(0);
    let lines: Vec<String> = d
        .interfaces
        .iter()
        .map(|i| {
            format!(
                "  {} {:w$}  {}",
                if i.picked { "+" } else { "-" },
                i.name,
                i.why
            )
        })
        .collect();
    Out::note(&format!(
        "Interfaces on this host (+ suggested):\n{}\nSuggested: {}",
        lines.join("\n"),
        if d.suggested.is_empty() {
            "none".to_owned()
        } else {
            d.suggested.join(" ")
        }
    ));
    let answer = ask(
        "Type yes for the suggestion, `all` to pick them again at each start, or interface names",
    )?;
    parse_choice(&answer, &d.suggested)
        .ok_or_else(|| Error::Usage("no interface chosen; nothing was set up".into()))
}

/// `iohr ext service NAME`: runs the set-up with sudo, which asks for the password.
fn service(store: &Store, name: &str, picks: &Picks) -> Result<(), Error> {
    if !cfg!(target_os = "linux") {
        return Err(Error::Usage(format!(
            "{name}'s system service runs on Linux only"
        )));
    }
    let program = service_program(store, name)?;
    let c = service_command(&program, &current_user()?, picks);
    Out::note(&format!("Running: {}", shown(&c)));
    let status = std::process::Command::new("sudo")
        .args(&c)
        .status()
        .map_err(|e| Error::Usage(format!("could not run sudo: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Usage(format!(
            "setting up {name}'s system service failed; run it again with `iohr ext service {name}`"
        )))
    }
}

/// After installing an extension with a system service: with --yes it is set up (with the
/// interfaces given, or the program's default), at a terminal the person picks the
/// interfaces from what the program detects, otherwise the command to run is printed.
fn offer_service(
    store: &Store,
    name: &str,
    picks: &Picks,
    yes: bool,
    ask: Option<&mut Ask<'_>>,
) -> Result<(), Error> {
    if !cfg!(target_os = "linux") {
        Out::note(&format!(
            "{name} runs as a system service on Linux; install it on a Linux host to set that up."
        ));
        return Ok(());
    }
    let program = service_program(store, name)?;
    if yes {
        return service(store, name, picks);
    }
    let Some(ask) = ask else {
        let line = shown(&service_command(&program, &current_user()?, picks));
        Out::note(&format!(
            "Not set up yet. Run `iohr ext service {name}` (or: {line}) when you are ready."
        ));
        return Ok(());
    };
    let picks = if *picks == Picks::Default {
        match ask_interfaces(store, name, Some(&mut *ask)) {
            Ok(p) => p,
            Err(e) => {
                Out::note(&format!(
                    "{e}. Run `iohr ext service {name}` when you are ready."
                ));
                return Ok(());
            }
        }
    } else {
        picks.clone()
    };
    let line = shown(&service_command(&program, &current_user()?, &picks));
    Out::note(&format!(
        "{name} runs as a system service. Setting it up runs, with sudo:\n  {line}"
    ));
    if ask("Type yes to set up its system service now")? == "yes" {
        service(store, name, &picks)
    } else {
        Out::note(&format!(
            "Not set up yet. Run `iohr ext service {name}` (or: {line}) when you are ready."
        ));
        Ok(())
    }
}

/// The privileges in plain words, one per line, for a note.
pub(super) fn privilege_lines(privileges: &[&str]) -> String {
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

    #[test]
    fn the_service_command_names_the_program_the_user_and_the_interface() {
        let p = std::path::Path::new(
            "/home/a/.local/share/iohr/extensions/capture/1.0.0-abc/iohr-capture",
        );
        let c = super::service_command(p, "nevio", &super::Picks::Default);
        assert_eq!(
            super::shown(&c),
            "sudo /home/a/.local/share/iohr/extensions/capture/1.0.0-abc/iohr-capture service install --agent-user nevio"
        );
        let named = super::Picks::Named(vec!["enp70s0".into(), "bond0".into()]);
        let c = super::service_command(p, "nevio", &named);
        assert!(
            super::shown(&c).ends_with("--agent-user nevio --interface enp70s0 --interface bond0")
        );
        let c = super::service_command(p, "nevio", &super::Picks::All);
        assert!(super::shown(&c).ends_with("--agent-user nevio --all"));
    }

    #[test]
    fn the_interface_answer_takes_the_suggestion_all_or_names() {
        use super::{Picks, parse_choice};
        let s = vec!["enp70s0".to_owned()];
        assert_eq!(parse_choice("yes", &s), Some(Picks::Named(s.clone())));
        assert_eq!(parse_choice(" all ", &s), Some(Picks::All));
        assert_eq!(
            parse_choice("enp70s0, cni0", &s),
            Some(Picks::Named(vec!["enp70s0".into(), "cni0".into()]))
        );
        assert_eq!(parse_choice("", &s), None);
        assert_eq!(parse_choice("no", &s), None);
        assert_eq!(parse_choice("yes", &[]), None);
    }
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
