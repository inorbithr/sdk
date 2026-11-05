//! `iohr memory remember|recall|list|forget|check|drift|sync`: a project's memory, local first
//! (RFC 0205, ADR 0216, RFC 0124.1).
//!
//! The first four work on files in `.inorbit/memory/` (or the person's own notes) with no
//! sign-in and no network. `check` and the code half of `drift` read the same files and git.
//! Only `sync`, `drift --remote` and the `account` scope talk to InOrbit's memory service
//! (`iohr.memory.v1.MemoryService`, RFC 0002), and only when `[memory] sync = true`, the flag
//! or the command asks for it.
//!
//! Memory text goes to stdout when the person asked to see it, in request bodies to the service
//! on a sync, and nowhere else: never a log, a URL or an error.

use std::path::{Path, PathBuf};

use inorbithr::Method;
use serde::Serialize;
use serde_json::{Value, json};

use crate::Env;
use crate::atlas;
use crate::cli::{Global, MemoryCommand};
use crate::context::{Ctx, Session, session};
use crate::error::Error;
use crate::front;
use crate::memory::remote::{self, Action, Remote};
use crate::memory::store::{self, Entry, Kind, Remembered, Scope, Store};
use crate::memory::{self, check, drift, recall};
use crate::output::Out;
use crate::roles;
use crate::ui::Ui;

use super::{day, pages, str_of};

pub(crate) async fn run(g: &Global, env: &Env, cmd: MemoryCommand, out: Out) -> Result<(), Error> {
    match cmd {
        MemoryCommand::Remember {
            text,
            why,
            refs,
            preference,
            kind,
            pin,
            scope,
        } => {
            let kind = match (kind.as_deref(), preference) {
                (Some(k), _) => Kind::parse(k),
                (None, true) => Kind::Preference,
                (None, false) => Kind::Fact,
            };
            let w = Where::resolve(g, scope.as_deref())?;
            remember(
                g,
                env,
                &w,
                &text,
                kind,
                why.as_deref().unwrap_or(""),
                &refs,
                pin,
                out,
            )
            .await
        }
        MemoryCommand::Recall {
            query,
            limit,
            scope,
        } => {
            let w = Where::resolve(g, scope.as_deref())?;
            recall_cmd(g, env, &w, &query, limit, out).await
        }
        MemoryCommand::List { limit, scope } => {
            let w = Where::resolve(g, scope.as_deref())?;
            list(g, env, &w, limit, out).await
        }
        MemoryCommand::Forget { id, scope } => {
            let w = Where::resolve(g, scope.as_deref())?;
            forget(g, env, &w, &id, out).await
        }
        MemoryCommand::Check {
            scope,
            strict,
            stale_days,
            checked_days,
        } => {
            let w = Where::resolve_local(g, scope.as_deref())?;
            let checked_days = match checked_days {
                Some(d) => d,
                None => Ctx::load(g)?
                    .config
                    .memory
                    .check_days
                    .unwrap_or(check::CHECKED_DAYS),
            };
            check_cmd(&w, strict, stale_days, checked_days, out)
        }
        MemoryCommand::Drift {
            scope,
            remote,
            strict,
        } => {
            let w = Where::resolve_local(g, scope.as_deref())?;
            drift_cmd(g, env, &w, remote, strict, out).await
        }
        MemoryCommand::Sync {
            scope,
            dry_run,
            pull,
        } => {
            let report = sync_for(g, env, scope.as_deref(), dry_run, pull).await?;
            show_sync(Ui::detect(out.json), &report, out);
            report.verdict()
        }
    }
}

// ---- where memory lives -------------------------------------------------------------------

/// Where a command's memory is: its scope, its folder, and the project it belongs to.
struct Where {
    scope: Scope,
    store: Option<Store>,
    /// The project root, when run inside one.
    root: Option<PathBuf>,
    /// The project's slug, for the source tag.
    slug: String,
    sync_on: bool,
}

impl Where {
    /// The scope of `--scope`, else `[memory] scope`, else the project's. A project scope
    /// chosen by default and not inside a project falls back to the user's own notes, said on
    /// stderr; asked for by `--scope project` it is an error.
    fn resolve(g: &Global, flag: Option<&str>) -> Result<Self, Error> {
        let config = Ctx::load(g)?.config;
        let sync_on = config.memory.sync_on();
        let wanted = match flag.or(config.memory.scope.as_deref()) {
            Some(s) => Scope::parse(s)?,
            None => Scope::Project,
        };
        let here = std::env::current_dir()
            .map_err(|e| Error::Failed(format!("no current folder: {e}")))?;
        let root = memory::find_root(&here);
        let slug = root
            .as_deref()
            .map(|r| atlas::slug(&super::atlas::project_meta(r).0))
            .unwrap_or_default();
        let scope = match wanted {
            Scope::Project if root.is_none() => {
                if flag.is_some() {
                    return Err(Error::Usage(
                        "`--scope project` needs a project: run it inside a folder with \
                         `.inorbit/`, `atlas/` or `.git`, or use `--scope user`"
                            .into(),
                    ));
                }
                Out::note("not inside a project: using your own notes (--scope user)");
                Scope::User
            }
            Scope::Account if !sync_on => {
                return Err(Error::Usage(
                    "account memory lives on InOrbit's service: set `[memory] sync = true` in \
                     the CLI config, or keep it here with `--scope project` or `--scope user`"
                        .into(),
                ));
            }
            s => s,
        };
        let store = match (scope, root.as_deref()) {
            (Scope::Project, Some(r)) => Some(Store::project(r)),
            (Scope::User, _) => Some(Store::user()?),
            _ => None,
        };
        Ok(Self {
            scope,
            store,
            root,
            slug,
            sync_on,
        })
    }

    /// As [`resolve`](Self::resolve), for commands that work on files only.
    fn resolve_local(g: &Global, flag: Option<&str>) -> Result<Self, Error> {
        let w = Self::resolve(g, flag)?;
        if w.scope == Scope::Account {
            return Err(Error::Usage(
                "this works on memories kept on this machine: use `--scope project` or \
                 `--scope user`"
                    .into(),
            ));
        }
        Ok(w)
    }

    fn store(&self) -> Result<&Store, Error> {
        self.store
            .as_ref()
            .ok_or_else(|| Error::Failed("no folder for this memory scope".into()))
    }

    /// The folder refs are relative to: the project's root, else here.
    fn base(&self) -> PathBuf {
        self.root
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default()
    }

    fn source(&self) -> String {
        remote::source_for(self.scope, &self.slug)
    }
}

fn now_unix() -> i64 {
    front::unix_of(&front::now()).unwrap_or(0)
}

/// The signed-in session, with a hint that says what the sign-in is for.
async fn remote_session(g: &Global, env: &Env, why: &str) -> Result<Session, Error> {
    session(g, env).await.map_err(|e| match e {
        Error::NotSignedIn(_) => Error::NotSignedIn(format!(
            "{why} needs a sign-in; memory on this machine does not: run `iohr login`"
        )),
        other => other,
    })
}

fn scoped(e: inorbithr::Error) -> Error {
    match e.status() {
        Some(401 | 403) => Error::with_hint(
            e,
            "Memory belongs to a signed-in person: run `iohr login`; an API key cannot use it.",
        ),
        Some(429) => Error::with_hint(
            e,
            "Your memory is full: `iohr memory list` and `iohr memory forget <id>` make room.",
        ),
        _ => e.into(),
    }
}

fn shown(path: &Path, root: Option<&Path>) -> String {
    root.and_then(|r| path.strip_prefix(r).ok())
        .unwrap_or(path)
        .display()
        .to_string()
}

// ---- remember, recall, list, forget -------------------------------------------------------

#[allow(clippy::too_many_arguments, reason = "the flags of one command")]
async fn remember(
    g: &Global,
    env: &Env,
    w: &Where,
    text: &str,
    kind: Kind,
    why: &str,
    refs: &[String],
    pin: bool,
    out: Out,
) -> Result<(), Error> {
    if w.scope == Scope::Account {
        return remember_remote(g, env, w, text, kind, out).await;
    }
    let store = w.store()?;
    let done = store::remember(store, text, kind, why, refs, pin)?;
    let (status, e, similar) = match done {
        Remembered::Created { entry, similar } => ("kept", entry, similar),
        Remembered::Confirmed(entry) => ("confirmed", entry, None),
    };
    if why.trim().is_empty() && e.why.is_empty() {
        Out::note("no --why: `iohr memory check` will flag a memory without a reason");
    }
    if let Some(other) = &similar {
        Out::note(&format!(
            "close to {other}: `iohr memory check` lists near duplicates"
        ));
    }
    let file = shown(&store.file(&e.id), w.root.as_deref());
    if out.json {
        Out::print_json(&json!({
            "status": status, "scope": w.scope, "file": file, "similar": similar, "entry": e,
        }));
    } else {
        Out::pairs(&[
            ("id", e.id.clone()),
            ("status", status.to_owned()),
            ("scope", w.scope.as_str().to_owned()),
            ("file", file),
        ]);
    }
    Ok(())
}

async fn remember_remote(
    g: &Global,
    env: &Env,
    place: &Where,
    text: &str,
    kind: Kind,
    out: Out,
) -> Result<(), Error> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > store::MAX_TEXT {
        return Err(Error::Usage(format!(
            "a memory is one short fact, 1 to {} characters",
            store::MAX_TEXT
        )));
    }
    if memory::text::has_secret(text) {
        return Err(Error::Usage(
            "that looks like a secret: nothing was kept".into(),
        ));
    }
    let sess = remote_session(g, env, "account memory").await?;
    let body = remote::remember_body(kind, text, &place.source());
    let reply: Value = sess
        .api
        .send(Method::Post, "/v1/memory/entries", &[], Some(&body))
        .await
        .and_then(|r| r.json())
        .map_err(scoped)?;
    let entry = reply.get("entry").cloned().unwrap_or(Value::Null);
    if out.json {
        Out::print_json(&reply);
    } else {
        Out::pairs(&[
            ("id", str_of(&entry, "id").to_owned()),
            ("scope", "account".to_owned()),
        ]);
    }
    Ok(())
}

fn local_cells(e: &Entry, score: Option<f64>) -> Vec<String> {
    vec![
        e.id.clone(),
        if e.pinned {
            format!("{} (pinned)", e.kind.as_str())
        } else {
            e.kind.as_str().to_owned()
        },
        day(&e.updated),
        score.map_or_else(String::new, |s| format!("{s:.2}")),
        e.text.clone(),
    ]
}

const LOCAL_HEAD: [&str; 5] = ["id", "kind", "updated", "score", "text"];

fn remote_cells(e: &Value) -> Vec<String> {
    let score = e
        .get("score")
        .and_then(Value::as_f64)
        .filter(|s| *s > 0.0)
        .map_or_else(String::new, |s| format!("{s:.2}"));
    vec![
        str_of(e, "id").to_owned(),
        Kind::from_api(str_of(e, "kind")).as_str().to_owned(),
        day(str_of(e, "created_at")),
        score,
        str_of(e, "text").to_owned(),
    ]
}

async fn recall_cmd(
    g: &Global,
    env: &Env,
    w: &Where,
    query: &str,
    limit: u32,
    out: Out,
) -> Result<(), Error> {
    if query.trim().is_empty() {
        return Err(Error::Usage(
            "say what to recall: the topic is empty".into(),
        ));
    }
    let want = limit.max(1) as usize;
    if w.scope == Scope::Account {
        let s = remote_session(g, env, "account memory").await?;
        let body = remote::recall_body(query.trim(), limit.max(1));
        let v: Value = s
            .api
            .send(Method::Post, "/v1/memory/recall", &[], Some(&body))
            .await
            .and_then(|r| r.json())
            .map_err(scoped)?;
        let entries: Vec<Value> = v
            .get("entries")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if out.json {
            Out::print_json(&json!({ "scope": "account", "entries": entries }));
        } else if entries.is_empty() {
            Out::note("nothing recalled");
        } else {
            let rows: Vec<Vec<String>> = entries.iter().map(remote_cells).collect();
            Out::table(&LOCAL_HEAD, &rows);
        }
        return Ok(());
    }
    let entries = w.store()?.load()?;
    let hits = recall::recall(&entries, query, want, now_unix());
    if out.json {
        let items: Vec<Value> = hits
            .iter()
            .map(|h| json!({ "score": h.score, "entry": h.entry }))
            .collect();
        Out::print_json(&json!({ "scope": w.scope, "entries": items }));
    } else if hits.is_empty() {
        Out::note("nothing recalled");
    } else {
        let rows: Vec<Vec<String>> = hits
            .iter()
            .map(|h| local_cells(&h.entry, Some(h.score)))
            .collect();
        Out::table(&LOCAL_HEAD, &rows);
    }
    Ok(())
}

async fn list(g: &Global, env: &Env, w: &Where, limit: usize, out: Out) -> Result<(), Error> {
    let limit = limit.max(1);
    if w.scope == Scope::Account {
        let s = remote_session(g, env, "account memory").await?;
        let mut all = pages(&s.api, "/v1/memory/entries", &[], "entries", limit)
            .await
            .map_err(scoped)?;
        all.truncate(limit);
        if out.json {
            Out::print_json(&json!({ "scope": "account", "entries": all }));
        } else if all.is_empty() {
            Out::note("nothing kept");
        } else {
            let rows: Vec<Vec<String>> = all.iter().map(remote_cells).collect();
            Out::table(&LOCAL_HEAD, &rows);
        }
        return Ok(());
    }
    let mut entries = w.store()?.load()?;
    entries.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| a.id.cmp(&b.id)));
    entries.truncate(limit);
    if out.json {
        Out::print_json(&json!({ "scope": w.scope, "entries": entries }));
    } else if entries.is_empty() {
        Out::note("nothing kept");
    } else {
        let rows: Vec<Vec<String>> = entries.iter().map(|e| local_cells(e, None)).collect();
        Out::table(&LOCAL_HEAD, &rows);
    }
    Ok(())
}

async fn delete_remote(s: &Session, id: &str) -> Result<(), Error> {
    s.api
        .send(
            Method::Delete,
            &format!("/v1/memory/entries/{id}"),
            &[],
            None,
        )
        .await
        .map_err(scoped)?;
    Ok(())
}

async fn forget(g: &Global, env: &Env, w: &Where, id: &str, out: Out) -> Result<(), Error> {
    if w.scope == Scope::Account {
        if !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || id.is_empty()
        {
            return Err(Error::Usage(
                "an entry id is letters, digits, _ and -".into(),
            ));
        }
        let s = remote_session(g, env, "account memory").await?;
        delete_remote(&s, id).await?;
        forgotten(id, w.scope, None, out);
        return Ok(());
    }
    if !store::valid_id(id) {
        return Err(Error::Usage(
            "a memory id is lower-case letters, digits and dashes, as `iohr memory list` shows"
                .into(),
        ));
    }
    let store = w.store()?;
    let entry = store.load()?.into_iter().find(|e| e.id == id);
    if !store.remove(id)? {
        return Err(Error::Usage(format!(
            "no memory `{id}` in the {} scope: `iohr memory list`",
            w.scope.as_str()
        )));
    }
    // A memory that was synced has a copy on the service. With sync on it goes too; with sync
    // off nothing is sent, and the person is told the copy stays.
    let note = match entry.and_then(|e| e.remote_id) {
        Some(rid) if w.sync_on => match remote_session(g, env, "deleting its copy").await {
            Ok(s) => match delete_remote(&s, &rid).await {
                Ok(()) => Some("its copy on the service is deleted too".to_owned()),
                Err(_) => Some(format!(
                    "the file is gone, but its copy on the service ({rid}) could not be deleted: \
                     `iohr memory forget {rid} --scope account` retries"
                )),
            },
            Err(_) => Some(format!(
                "the file is gone; its copy on the service ({rid}) stays until you sign in and \
                 run `iohr memory forget {rid} --scope account`"
            )),
        },
        Some(rid) => Some(format!(
            "its copy on the service ({rid}) stays: sync is off, nothing was sent"
        )),
        None => None,
    };
    forgotten(id, w.scope, note.as_deref(), out);
    Ok(())
}

fn forgotten(id: &str, scope: Scope, note: Option<&str>, out: Out) {
    if let Some(n) = note {
        Out::note(n);
    }
    if out.json {
        Out::print_json(&json!({ "forgotten": id, "scope": scope, "note": note }));
    } else {
        Out::pairs(&[("forgotten", id.to_owned())]);
    }
}

// ---- check --------------------------------------------------------------------------------

fn check_cmd(
    w: &Where,
    strict: bool,
    stale_days: u32,
    checked_days: u32,
    out: Out,
) -> Result<(), Error> {
    let entries = w.store()?.load()?;
    let base = w.base();
    let lang = roles::detect_language(&base);
    let mut findings = check::check(&entries, &base, lang, now_unix(), stale_days, checked_days);
    // Topics a project is expected to remember are a project's business, not a person's notes.
    if w.scope != Scope::Project {
        findings.retain(|f| f.rule != "gap");
    }
    if out.json {
        Out::print_json(&json!({
            "scope": w.scope, "memories": entries.len(), "findings": findings,
        }));
    } else if findings.is_empty() {
        Ui::detect(false).outro(format!("{} memories, nothing found", entries.len()));
    } else {
        let rows: Vec<Vec<String>> = findings
            .iter()
            .map(|f| vec![f.id.clone(), f.rule.to_owned(), f.message.clone()])
            .collect();
        Out::table(&["id", "rule", "finding"], &rows);
    }
    if strict && !findings.is_empty() {
        return Err(Error::Findings(format!(
            "{} finding{} in {} memories",
            findings.len(),
            if findings.len() == 1 { "" } else { "s" },
            entries.len()
        )));
    }
    Ok(())
}

// ---- drift --------------------------------------------------------------------------------

async fn drift_cmd(
    g: &Global,
    env: &Env,
    w: &Where,
    remote_flag: bool,
    strict: bool,
    out: Out,
) -> Result<(), Error> {
    let entries = w.store()?.load()?;
    let base = w.base();
    let code = drift::code(&entries, &base);
    let service = if remote_flag || w.sync_on {
        let s = remote_session(g, env, "comparing with the service").await?;
        let all = fetch_remote(&s, &w.source()).await?;
        Some(remote::plan(&entries, &all))
    } else {
        None
    };
    let moved: Vec<&Action> = service
        .iter()
        .flatten()
        .filter(|a| !matches!(a, Action::InSync { .. }))
        .collect();
    if out.json {
        Out::print_json(&json!({
            "scope": w.scope,
            "code": code,
            "service": service,
        }));
    } else {
        if let Some(why) = &code.skipped {
            Out::note(&format!("code: skipped, {why}"));
        } else if code.drifts.is_empty() {
            Out::note("code: no memory's files changed since it was confirmed");
        } else {
            let rows: Vec<Vec<String>> = code
                .drifts
                .iter()
                .map(|d| {
                    vec![
                        d.id.clone(),
                        d.rule.to_owned(),
                        d.path.clone(),
                        d.message.clone(),
                    ]
                })
                .collect();
            Out::table(&["id", "rule", "path", "finding"], &rows);
        }
        if service.is_some() {
            if moved.is_empty() {
                Out::note("service: every memory is the same on both sides");
            } else {
                Out::table(&["subject", "against the service"], &action_rows(&moved));
            }
        }
    }
    if strict && (!code.drifts.is_empty() || !moved.is_empty()) {
        return Err(Error::Findings(format!(
            "{} memory ref(s) and {} service difference(s) drifted",
            code.drifts.len(),
            moved.len()
        )));
    }
    Ok(())
}

fn action_rows(actions: &[&Action]) -> Vec<Vec<String>> {
    actions
        .iter()
        .map(|a| {
            let note = match a {
                Action::Conflict { why, .. } => format!("conflict: {why}"),
                other => other.name().to_owned(),
            };
            vec![a.subject().to_owned(), note]
        })
        .collect()
}

/// Every entry of the account's memory whose source is `source`.
async fn fetch_remote(s: &Session, source: &str) -> Result<Vec<Remote>, Error> {
    let all = pages(&s.api, "/v1/memory/entries", &[], "entries", 10_000)
        .await
        .map_err(scoped)?;
    Ok(remote::keep(
        all.iter().filter_map(Remote::from_value).collect(),
        source,
    ))
}

// ---- sync ---------------------------------------------------------------------------------

/// One line of a sync's report.
#[derive(Debug, Serialize)]
pub(crate) struct Row {
    pub id: String,
    pub action: String,
    pub note: String,
}

/// What a sync did, or would do.
#[derive(Debug, Serialize)]
pub(crate) struct SyncReport {
    pub scope: Scope,
    pub source: String,
    pub dry_run: bool,
    pub rows: Vec<Row>,
    pub conflicts: usize,
}

impl SyncReport {
    /// A conflict is a finding: the exit code says something is left to resolve.
    pub(crate) fn verdict(&self) -> Result<(), Error> {
        if self.conflicts > 0 {
            return Err(Error::Findings(format!(
                "{} {} changed on both sides: nothing was overwritten; `iohr memory drift \
                 --remote` shows them",
                self.conflicts,
                if self.conflicts == 1 {
                    "memory"
                } else {
                    "memories"
                }
            )));
        }
        Ok(())
    }
}

fn row(id: &str, action: &str, note: &str) -> Row {
    Row {
        id: id.to_owned(),
        action: action.to_owned(),
        note: note.to_owned(),
    }
}

pub(crate) fn show_sync(ui: Ui, r: &SyncReport, out: Out) {
    if out.json {
        Out::print_json(r);
        return;
    }
    if r.rows.is_empty() {
        ui.outro("no memories to sync");
    } else {
        let cells: Vec<Vec<String>> = r
            .rows
            .iter()
            .map(|x| vec![x.id.clone(), x.action.clone(), x.note.clone()])
            .collect();
        Out::table(&["memory", "action", "note"], &cells);
        if r.dry_run {
            ui.outro("dry run: nothing was sent");
        } else if r.conflicts == 0 {
            ui.outro("synced");
        }
    }
}

/// Sync the memories of `flag`'s scope (else the config's) with the service. Nothing is
/// overwritten on either side: a change on both sides is a conflict, reported.
#[allow(
    clippy::too_many_lines,
    reason = "one decision table, one arm per action"
)]
pub(crate) async fn sync_for(
    g: &Global,
    env: &Env,
    flag: Option<&str>,
    dry: bool,
    pull: bool,
) -> Result<SyncReport, Error> {
    let mut place = Where::resolve(g, flag)?;
    if place.scope == Scope::Account {
        return Err(Error::Usage(
            "account memory is already on the service: sync works on `--scope project` or \
             `--scope user`"
                .into(),
        ));
    }
    // Running sync is the person's choice, whatever the config says.
    place.sync_on = true;
    let store = place.store()?.clone();
    let source = place.source();
    let mut local = store.load()?;
    let sess = remote_session(g, env, "memory sync").await?;
    let all = fetch_remote(&sess, &source).await?;
    let by_id: std::collections::HashMap<String, Remote> = all
        .iter()
        .map(|found| (found.id.clone(), found.clone()))
        .collect();
    let plan = remote::plan(&local, &all);
    let mut rows = Vec::new();
    let mut conflicts = 0;
    for a in &plan {
        match a {
            Action::InSync { .. } => {}
            Action::Conflict { id, why } => {
                conflicts += 1;
                rows.push(row(id, "conflict", why));
            }
            Action::RemoteNewer { id, remote_id } => rows.push(row(
                id,
                "remote newer",
                &format!("the service's copy ({remote_id}) differs; not pulled over the file"),
            )),
            Action::RemoteGone { id } => rows.push(row(
                id,
                "remote gone",
                "the service no longer has it; `iohr memory sync` would push it again after an edit",
            )),
            Action::Link { id, remote_id } => {
                if dry {
                    rows.push(row(id, "would link", remote_id));
                } else if let (Some(mem), Some(found)) = (
                    local.iter_mut().find(|mem| &mem.id == id),
                    by_id.get(remote_id),
                ) {
                    remote::link(mem, found);
                    store.save(mem)?;
                    rows.push(row(id, "linked", remote_id));
                }
            }
            Action::Push { id } | Action::Replace { id, .. } => {
                let old = match a {
                    Action::Replace { remote_id, .. } => Some(remote_id.clone()),
                    _ => None,
                };
                let verb = if old.is_some() { "replace" } else { "push" };
                let Some(mem) = local.iter_mut().find(|mem| &mem.id == id) else {
                    continue;
                };
                if memory::text::has_secret(&mem.text) || memory::text::has_secret(&mem.why) {
                    rows.push(row(id, "blocked", "looks like a secret: not sent"));
                    continue;
                }
                if dry {
                    rows.push(row(id, &format!("would {verb}"), ""));
                    continue;
                }
                let body = remote::remember_body(mem.kind, &mem.text, &source);
                let reply: Value = sess
                    .api
                    .send(Method::Post, "/v1/memory/entries", &[], Some(&body))
                    .await
                    .and_then(|found| found.json())
                    .map_err(scoped)?;
                let Some(created) = reply.get("entry").and_then(Remote::from_value) else {
                    return Err(Error::Failed(
                        "the service answered without an entry: nothing was linked".into(),
                    ));
                };
                // The new id is written to the file before the old copy goes, so an
                // interrupted sync can leave a duplicate on the service, never lose a memory.
                let mut created = created;
                created.kind = mem.kind;
                created.text.clone_from(&mem.text);
                remote::link(mem, &created);
                store.save(mem)?;
                let mut note = created.id.clone();
                if let Some(old) = old {
                    match delete_remote(&sess, &old).await {
                        Ok(()) => note = format!("{} (replaced {old})", created.id),
                        Err(_) => {
                            note = format!(
                                "{} (the old copy {old} could not be deleted: forget it with \
                                 --scope account)",
                                created.id
                            );
                        }
                    }
                }
                rows.push(row(id, if verb == "push" { "pushed" } else { "replaced" }, &note));
            }
            Action::RemoteOnly { remote_id } => {
                if !pull {
                    rows.push(row(
                        remote_id,
                        "remote only",
                        "on the service, no file here: `--pull` writes it",
                    ));
                    continue;
                }
                let Some(found) = by_id.get(remote_id) else {
                    continue;
                };
                let mem = remote::pulled(found);
                if local.iter().any(|l| l.id == mem.id) {
                    rows.push(row(remote_id, "skipped", "a file with the same text exists"));
                } else if dry {
                    rows.push(row(remote_id, "would pull", &mem.id));
                } else {
                    store.save(&mem)?;
                    rows.push(row(remote_id, "pulled", &mem.id));
                    local.push(mem);
                }
            }
        }
    }
    Ok(SyncReport {
        scope: place.scope,
        source,
        dry_run: dry,
        rows,
        conflicts,
    })
}
