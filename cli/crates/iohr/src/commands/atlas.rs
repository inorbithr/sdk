//! `iohr atlas init|new|sync` and `iohr sync` (RFC 0205): the project's own PRDs, RFCs, ADRs
//! and studies in `atlas/`, and the push of them to its Decisions space.
//!
//! Nothing leaves the machine unless the person runs `sync`. The output names files, ids and
//! versions; it never prints a document's text or a finding's message.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::Env;
use crate::atlas::{self, Kind};
use crate::cli::{AtlasCommand, AtlasKind, Global, RolesCommand, SyncArgs};
use crate::commands::decisions::{create_in, decisions, save_text, scoped};
use crate::context::{Api, Ctx, session};
use crate::error::Error;
use crate::output::Out;
use crate::roles::{self, Report};
use crate::ui::{Mark, Ui};
use inorbithr::public::DecisionsGetDocumentParams as GetDocumentParams;

const MESSAGE: &str = "Synced from atlas by iohr sync";

pub(crate) async fn run(g: &Global, env: &Env, cmd: AtlasCommand, out: Out) -> Result<(), Error> {
    match cmd {
        AtlasCommand::Init { dir } => init(g, env, dir, out).await,
        AtlasCommand::New { kind, title, dir } => new(kind, &title, dir, out),
        AtlasCommand::Sync(args) => sync(g, env, &args, out).await,
        AtlasCommand::Roles(cmd) => roles_cmd(g, env, cmd, out).await,
        AtlasCommand::Search {
            query,
            site,
            limit,
            dir,
        } => kb_search(&query, site.as_deref(), limit, dir, out),
        AtlasCommand::Facts {
            site,
            predicate,
            grep,
            limit,
            dir,
        } => kb_facts(
            (site.as_deref(), predicate.as_deref(), grep.as_deref()),
            limit,
            dir,
            out,
        ),
        AtlasCommand::Index {
            pages,
            site,
            limit,
            dir,
        } => kb_index(pages, site.as_deref(), limit, dir, out),
    }
}

fn kb_site(site: Option<&str>) -> Option<String> {
    site.map(crate::kb::site_name)
}

/// `iohr atlas search`: the best chunks for a query, with their sources.
fn kb_search(
    query: &str,
    site: Option<&str>,
    limit: usize,
    dir: Option<PathBuf>,
    out: Out,
) -> Result<(), Error> {
    let root = root_of(dir)?;
    let site = kb_site(site);
    let hits = crate::kb::search(&root, query, site.as_deref(), limit.clamp(1, 50))
        .map_err(Error::Failed)?;
    if out.json {
        let items: Vec<_> = hits
            .iter()
            .map(|h| {
                serde_json::json!({
                    "chunk": h.id, "site": h.site, "url": h.url, "title": h.title,
                    "heading": h.heading, "path": h.path, "start": h.start, "end": h.end,
                    "tokens": h.tokens, "score": h.score, "text": h.text,
                })
            })
            .collect();
        Out::print_json(&serde_json::json!({
            "query": query, "embeddings": crate::kb::embed::STATUS, "hits": items,
        }));
        return Ok(());
    }
    if hits.is_empty() {
        Out::note(&format!("nothing in the knowledge base matches {query:?}"));
        return Ok(());
    }
    for (i, h) in hits.iter().enumerate() {
        let path = if h.path.is_empty() { &h.title } else { &h.path };
        let text: String = h.text.split_whitespace().collect::<Vec<_>>().join(" ");
        let shown: String = text.chars().take(280).collect();
        let more = if text.chars().count() > 280 {
            " ..."
        } else {
            ""
        };
        Out::note(&format!(
            "{n}. {path}   [{tokens} words, score {score:.2}]\n   {url}   chars {s}-{e}   {id}\n   {shown}{more}",
            n = i + 1,
            tokens = h.tokens,
            score = h.score,
            url = h.url,
            s = h.start,
            e = h.end,
            id = h.id,
        ));
    }
    Ok(())
}

/// `iohr atlas facts`: what the site model says, with the chunk each fact came from.
fn kb_facts(
    (site, predicate, grep): (Option<&str>, Option<&str>, Option<&str>),
    limit: usize,
    dir: Option<PathBuf>,
    out: Out,
) -> Result<(), Error> {
    let root = root_of(dir)?;
    let site = kb_site(site);
    let rows = crate::kb::facts(&root, site.as_deref(), predicate, grep, limit.clamp(1, 500))
        .map_err(Error::Failed)?;
    if out.json {
        let items: Vec<_> = rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.fact.id, "site": r.site, "subject": r.fact.subject,
                    "predicate": r.fact.predicate, "value": r.fact.value, "url": r.fact.url,
                    "chunk": r.fact.chunk, "source": r.fact.source,
                })
            })
            .collect();
        Out::print_json(&serde_json::json!({ "facts": items }));
        return Ok(());
    }
    if rows.is_empty() {
        Out::note("no facts match");
        return Ok(());
    }
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.site.clone(),
                r.fact.predicate.clone(),
                r.fact.value.clone(),
                r.fact.url.clone(),
                r.fact.source.clone(),
            ]
        })
        .collect();
    Out::table(&["site", "predicate", "value", "page", "source"], &cells);
    Ok(())
}

/// `iohr atlas index`: the crawled sites (or, with `--pages`, their pages).
fn kb_index(
    pages: bool,
    site: Option<&str>,
    limit: usize,
    dir: Option<PathBuf>,
    out: Out,
) -> Result<(), Error> {
    let root = root_of(dir)?;
    let site = kb_site(site);
    if pages {
        let rows = crate::kb::pages(&root, site.as_deref(), limit.clamp(1, 1000))
            .map_err(Error::Failed)?;
        if out.json {
            let items: Vec<_> = rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "site": r.site, "url": r.url, "title": r.title, "lang": r.lang,
                        "chunks": r.chunks, "tokens": r.tokens,
                    })
                })
                .collect();
            Out::print_json(&serde_json::json!({ "pages": items }));
            return Ok(());
        }
        let cells: Vec<Vec<String>> = rows
            .iter()
            .map(|r| {
                vec![
                    r.url.clone(),
                    r.title.clone(),
                    r.lang.clone(),
                    r.chunks.to_string(),
                    r.tokens.to_string(),
                ]
            })
            .collect();
        Out::table(&["page", "title", "lang", "chunks", "words"], &cells);
        return Ok(());
    }
    let mut sites = crate::kb::status(&root).map_err(Error::Failed)?;
    if let Some(s) = &site {
        sites.retain(|x| &x.site == s);
    }
    if out.json {
        let items: Vec<_> = sites
            .iter()
            .map(|s| {
                let kinds = crate::kb::entity_kinds(&root, &s.site).unwrap_or_default();
                serde_json::json!({
                    "site": s.site, "subject": s.subject, "crawled": s.crawled,
                    "built": s.built, "embeddings": s.embeddings, "pages": s.pages,
                    "chunks": s.chunks, "tokens": s.tokens, "entities": s.entities,
                    "entity_kinds": kinds.into_iter().collect::<std::collections::BTreeMap<_, _>>(),
                    "facts": s.facts, "links": s.links,
                    "sections": s.sections.iter().map(|x| serde_json::json!({
                        "name": x.name, "pages": x.pages, "chunks": x.chunks,
                        "tokens": x.tokens, "summary": x.summary,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        Out::print_json(&serde_json::json!({ "sites": items }));
        return Ok(());
    }
    if sites.is_empty() {
        Out::note("no site is indexed; run `iohr project init --crawl`");
        return Ok(());
    }
    let cells: Vec<Vec<String>> = sites
        .iter()
        .map(|s| {
            vec![
                s.site.clone(),
                s.pages.to_string(),
                s.chunks.to_string(),
                s.tokens.to_string(),
                s.entities.to_string(),
                s.facts.to_string(),
                s.links.to_string(),
                s.built.clone(),
            ]
        })
        .collect();
    Out::table(
        &[
            "site", "pages", "chunks", "words", "entities", "facts", "links", "built",
        ],
        &cells,
    );
    Ok(())
}

fn start(dir: Option<PathBuf>) -> Result<PathBuf, Error> {
    match dir {
        Some(d) => Ok(d),
        None => {
            std::env::current_dir().map_err(|e| Error::Failed(format!("no current folder: {e}")))
        }
    }
}

/// The project's root: `--dir` (or here) and the nearest folder up that has `atlas/`.
fn root_of(dir: Option<PathBuf>) -> Result<PathBuf, Error> {
    let from = start(dir)?;
    atlas::find_root(&from).ok_or_else(|| {
        Error::Usage(format!(
            "no atlas/ folder in {} or above it; `iohr atlas init` makes one",
            from.display()
        ))
    })
}

fn kind_of(k: AtlasKind) -> Kind {
    match k {
        AtlasKind::Prd => Kind::Prd,
        AtlasKind::Rfc => Kind::Rfc,
        AtlasKind::Adr => Kind::Adr,
        AtlasKind::Study => Kind::Study,
    }
}

/// The name and template of a project from its `inorbit.toml`, else the folder's name.
pub(crate) fn project_meta(root: &Path) -> (String, String) {
    let table = std::fs::read_to_string(root.join("inorbit.toml"))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok());
    let pick = |key: &str| {
        table
            .as_ref()
            .and_then(|t| t.get(key))
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
    };
    let name = pick("name").unwrap_or_else(|| {
        root.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_owned()
    });
    (name, pick("template").unwrap_or_else(|| "none".to_owned()))
}

/// Moves an older `atlas/personalities/` to `atlas/roles/` and says so in one line (not in
/// `--json`, which the caller adds to its own object). Runs before anything is written.
pub(crate) fn move_legacy_roles(root: &Path, out: Out) -> Result<Option<String>, Error> {
    let moved = roles::migrate_legacy(root).map_err(Error::Failed)?;
    if let Some(line) = moved.as_deref().filter(|_| !out.json) {
        Ui::detect(false).info(line);
    }
    Ok(moved)
}

async fn init(g: &Global, env: &Env, dir: Option<PathBuf>, out: Out) -> Result<(), Error> {
    let root = start(dir)?;
    if !root.is_dir() {
        return Err(Error::Usage(format!("{} is not a folder", root.display())));
    }
    let moved = move_legacy_roles(&root, out)?;
    let (name, template) = project_meta(&root);
    let created_at = super::project::now();
    let bin = super::shell::running_bin();
    // The language the project declares, else the one its build file shows.
    let lang = roles::detect_language(&root);
    let mut written = atlas::ensure(
        &root,
        &atlas::Meta {
            name: &name,
            template: &template,
            lang,
            describe: None,
            created_at: &created_at,
            bin: &bin,
        },
    )
    .map_err(Error::Failed)?;
    if roles::ensure_record(&root, lang).map_err(Error::Failed)? {
        written.push("inorbit.toml".to_owned());
    }
    written.extend(roles::ensure_agents(&root, &bin).map_err(Error::Failed)?);
    let report = import_for(g, env, &root, false, false).await;
    if out.json {
        Out::print_json(&serde_json::json!({
            "root": root.display().to_string(),
            "written": written,
            "language": lang,
            "moved": moved,
            "roles": report,
        }));
        return Ok(());
    }
    let ui = Ui::detect(false);
    show_report(ui, &report);
    if written.is_empty() {
        ui.outro("atlas/ is already complete: nothing written");
    } else {
        ui.note("Written", &written.join("\n"));
        ui.outro("atlas/ ready. Documents stay on this machine until you run `iohr sync`.");
    }
    Ok(())
}

/// Imports the project's roles when signed in; never fails (a failed import is
/// in the report, and `iohr sync` tries again).
pub(crate) async fn import_for(
    g: &Global,
    env: &Env,
    root: &Path,
    dry: bool,
    upgrade: bool,
) -> Report {
    let session = session(g, env).await.ok();
    roles::import(session.as_ref().map(|s| &s.api), root, dry, upgrade).await
}

/// One line per role, then the one line about what could not happen.
pub(crate) fn show_report(ui: Ui, report: &Report) {
    let lines = report.lines();
    if lines.is_empty() {
        return;
    }
    ui.note("Roles", &lines.join("\n"));
}

/// How the roles step of `init` ended, in one line: imported (a check), or kept in the folder
/// only because nobody is signed in or a role did not go (a warning, with the fix `message` says).
pub(crate) fn roles_result(report: &Report) -> (Mark, String) {
    let names = |failed: bool| {
        report
            .outcomes
            .iter()
            .filter(|o| (o.action == "failed") == failed)
            .map(|o| o.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let listed = |failed: bool| {
        let n = names(failed);
        if n.is_empty() {
            String::new()
        } else {
            format!(" ({n})")
        }
    };
    let fix = report.message.clone().unwrap_or_else(|| {
        "the roles stay in atlas/roles/; run `iohr login`, then `iohr sync` imports them".to_owned()
    });
    if report.outcomes.iter().any(|o| o.action == "failed") {
        (
            Mark::Warn,
            format!("Roles not imported{}: {fix}", listed(true)),
        )
    } else if report.signed_in {
        (Mark::Ok, format!("Roles in your account: {}", names(false)))
    } else {
        (
            Mark::Warn,
            format!("Roles kept in atlas/roles/{}: {fix}", listed(false)),
        )
    }
}

async fn roles_cmd(g: &Global, env: &Env, cmd: RolesCommand, out: Out) -> Result<(), Error> {
    match cmd {
        RolesCommand::List { dir } => {
            let root = root_of(dir)?;
            move_legacy_roles(&root, out)?;
            let found = roles::locals(&root);
            let rows: Vec<serde_json::Value> = found
                .iter()
                .map(|f| {
                    let p = &f.role;
                    serde_json::json!({
                        "name": p.name,
                        "parent": p.parent,
                        "language": p.language,
                        "core_tokens": roles::core_tokens(p),
                        "lines": p.lines.len(),
                        "catalogue_version": p.version,
                        "imported_version": f.imported_version,
                    })
                })
                .collect();
            if out.json {
                Out::print_json(&serde_json::json!({ "roles": rows }));
                return Ok(());
            }
            if rows.is_empty() {
                Ui::detect(false).outro("no roles in atlas/roles/: `iohr atlas init` writes them");
                return Ok(());
            }
            let cells: Vec<Vec<String>> = found
                .iter()
                .map(|f| {
                    let p = &f.role;
                    vec![
                        p.name.clone(),
                        p.parent.clone(),
                        roles::core_tokens(p).to_string(),
                        p.lines.len().to_string(),
                        f.imported_version
                            .map_or_else(|| "not imported".to_owned(), |v| format!("v{v}")),
                    ]
                })
                .collect();
            Out::table(
                &["name", "parent", "core tokens", "lines", "in InOrbit"],
                &cells,
            );
            Ok(())
        }
        RolesCommand::Import {
            dry_run,
            upgrade,
            dir,
        } => {
            let root = root_of(dir)?;
            move_legacy_roles(&root, out)?;
            let report = import_for(g, env, &root, dry_run, upgrade).await;
            if out.json {
                Out::print_json(&serde_json::json!({ "dry_run": dry_run, "roles": report }));
                return Ok(());
            }
            let ui = Ui::detect(false);
            show_report(ui, &report);
            if dry_run {
                if let Some(b) = &report.body {
                    Out::print_json(b);
                }
                ui.outro("dry run: nothing was sent");
            }
            Ok(())
        }
    }
}

fn new(kind: AtlasKind, title: &str, dir: Option<PathBuf>, out: Out) -> Result<(), Error> {
    let root = root_of(dir)?;
    let date = super::project::now();
    let date = date.get(..10).unwrap_or(&date);
    let (rel, number) =
        atlas::new_document(&root, kind_of(kind), title, date).map_err(Error::Usage)?;
    if out.json {
        Out::print_json(&serde_json::json!({ "file": rel, "number": number }));
    } else {
        Ui::detect(false).outro(format!("{rel} written (number {number:04})"));
    }
    Ok(())
}

/// One document's result in a sync run.
#[derive(Debug, Clone, Serialize)]
struct Row {
    file: String,
    kind: &'static str,
    action: &'static str,
    id: String,
    version: Option<i32>,
    note: String,
}

impl Row {
    fn new(
        f: &atlas::Found,
        action: &'static str,
        id: &str,
        version: Option<i32>,
        note: &str,
    ) -> Self {
        Self {
            file: f.rel.clone(),
            kind: f.kind.api(),
            action,
            id: id.to_owned(),
            version,
            note: note.to_owned(),
        }
    }

    fn cells(&self) -> Vec<String> {
        vec![
            self.file.clone(),
            self.kind.to_owned(),
            self.action.to_owned(),
            self.id.clone(),
            self.version.map_or_else(String::new, |v| v.to_string()),
            self.note.clone(),
        ]
    }
}

fn int_of(text: &str, key: &str) -> Option<i32> {
    atlas::get(text, key).and_then(|v| v.trim().parse().ok())
}

fn write_back(f: &atlas::Found, keys: &[(&str, String)]) -> Result<(), Error> {
    // From the file as it is now, so a second write-back keeps what the first one recorded.
    let now = std::fs::read_to_string(&f.path)
        .map_err(|e| Error::Failed(format!("cannot read {}: {e}", f.rel)))?;
    std::fs::write(&f.path, atlas::set_keys(&now, keys))
        .map_err(|e| Error::Failed(format!("cannot write {}: {e}", f.rel)))
}

/// What a document is, as Decisions should get it, without the document's own words in any
/// message.
struct Plan {
    text: String,
    sha: String,
}

fn plan_of(f: &atlas::Found) -> Plan {
    Plan {
        text: atlas::strip_local(&f.text),
        sha: atlas::fingerprint(&f.text),
    }
}

/// The session, opened when the first remote call needs it.
struct Lazy<'a> {
    g: &'a Global,
    env: &'a Env,
    api: Option<Api>,
}

impl Lazy<'_> {
    async fn api(&mut self) -> Result<&Api, Error> {
        if self.api.is_none() {
            self.api = Some(session(self.g, self.env).await?.api);
        }
        self.api
            .as_ref()
            .ok_or_else(|| Error::Failed("no session".into()))
    }
}

/// `iohr sync`: the atlas documents and, when `[memory] sync = true`, the project's memories
/// (`iohr memory sync`). Each stops on a conflict and overwrites nothing.
pub(crate) async fn sync(g: &Global, env: &Env, args: &SyncArgs, out: Out) -> Result<(), Error> {
    let memory = if Ctx::load(g)?.config.memory.sync_on() {
        let report = super::memory::sync_for(g, env, None, args.dry_run, false).await?;
        if !out.json {
            super::memory::show_sync(Ui::detect(false), &report, out);
        }
        Some(report)
    } else {
        None
    };
    let docs = sync_documents(g, env, args, out, memory.as_ref()).await;
    if let (true, true, Some(r)) = (docs.is_err(), out.json, &memory) {
        Out::print_json(r);
    }
    docs?;
    memory.map_or(Ok(()), |r| r.verdict())
}

async fn sync_documents(
    g: &Global,
    env: &Env,
    args: &SyncArgs,
    out: Out,
    memory: Option<&super::memory::SyncReport>,
) -> Result<(), Error> {
    let root = root_of(args.dir.clone())?;
    let config = Ctx::load(g)?.config;
    let ui = Ui::detect(out.json);
    if config.sync.background_on() && !out.json {
        ui.info(
            "[sync] background is on, but this build has no background loop yet: \
             `iohr sync` is the only way a document leaves the machine.",
        );
    }
    let space = args
        .space
        .clone()
        .or_else(|| crate::project::space::read_link_for(&root, g.environment_name()).map(|(id, _)| id))
        .ok_or_else(|| {
            Error::Usage(
                "no Decisions space: give `--space ID`, or link the project with `/inorbit:link <space>`"
                    .into(),
            )
        })?;
    move_legacy_roles(&root, out)?;
    let found = atlas::scan(&root);
    let mut lazy = Lazy { g, env, api: None };
    let mut rows = Vec::new();
    for f in &found {
        rows.push(sync_one(&mut lazy, &space, f, args.dry_run).await?);
    }
    let local = atlas::local_only(&root);
    // The built-in roles go with the documents; a failed import never fails the sync.
    let report = import_for(g, env, &root, args.dry_run, false).await;
    if out.json {
        Out::print_json(&serde_json::json!({
            "space": space,
            "dry_run": args.dry_run,
            "documents": rows,
            "local_only": local,
            "roles": report,
            "memory": memory,
        }));
        return Ok(());
    }
    show_report(ui, &report);
    if rows.is_empty() {
        ui.outro("no documents in atlas/ to sync");
        return Ok(());
    }
    let cells: Vec<Vec<String>> = rows.iter().map(Row::cells).collect();
    Out::table(&["file", "kind", "action", "id", "version", "note"], &cells);
    if !local.is_empty() {
        ui.info(&format!("stays local, never synced: {}", local.join(", ")));
    }
    let conflicts = rows.iter().filter(|r| r.action == "conflict").count();
    if args.dry_run {
        ui.outro("dry run: nothing was sent");
    } else if conflicts > 0 {
        ui.warn(&format!(
            "{conflicts} document(s) changed in Decisions too: nothing was overwritten; \
             `iohr decisions show <id> --text --raw` gives the newer text"
        ));
    } else {
        ui.outro("synced");
    }
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "one decision table: create, skip, save, conflict"
)]
async fn sync_one(
    lazy: &mut Lazy<'_>,
    space: &str,
    f: &atlas::Found,
    dry: bool,
) -> Result<Row, Error> {
    let plan = plan_of(f);
    let remote_id = atlas::get(&f.text, "remote_id").filter(|s| !s.is_empty());
    let Some(id) = remote_id else {
        // New here: create it, then save the text on the version the create returned.
        if dry {
            return Ok(Row::new(f, "would create", "", None, ""));
        }
        let title = atlas::get(&f.text, "title").unwrap_or_else(|| "Untitled".to_owned());
        let summary = atlas::get(&f.text, "summary").filter(|s| !s.is_empty());
        let api = lazy.api().await?;
        let made = create_in(api, space, f.kind.api(), &title, summary, None).await?;
        let Some(doc) = made.document else {
            return Err(Error::Failed("Decisions made no document".into()));
        };
        let number = doc.number.to_string();
        // Record the id first: a failed save must never lead to a second create.
        write_back(
            f,
            &[
                ("remote_id", doc.document_id.clone()),
                ("remote_version", doc.current_version.to_string()),
                ("remote_number", number.clone()),
                ("remote_sha", String::new()),
            ],
        )?;
        let saved = save_text(
            api,
            space,
            &doc.document_id,
            doc.current_version,
            plan.text,
            MESSAGE,
        )
        .await?;
        let version = saved
            .document
            .as_ref()
            .map_or(saved.version, |d| d.current_version);
        write_back(
            f,
            &[
                ("remote_version", version.to_string()),
                ("remote_sha", plan.sha),
            ],
        )?;
        return Ok(Row::new(f, "created", &doc.document_id, Some(version), ""));
    };

    let local_version = int_of(&f.text, "remote_version").unwrap_or(0);
    let unchanged = atlas::get(&f.text, "remote_sha").is_some_and(|s| s == plan.sha);
    // A document we have no changed text for needs no remote read to be skipped, but the
    // remote may have moved on: read it to say so.
    let api = lazy.api().await?;
    let remote = match decisions(api)
        .get_document(space, &id, &GetDocumentParams::default())
        .await
    {
        Ok(r) => r.value,
        Err(e) if e.status() == Some(404) => {
            return Ok(Row::new(
                f,
                "missing remote",
                &id,
                None,
                "not in the space: nothing recreated",
            ));
        }
        Err(e) => return Err(scoped(e)),
    };
    let remote_version = remote
        .document
        .as_ref()
        .map_or(remote.version, |d| d.current_version);
    if remote_version > local_version {
        return Ok(if unchanged {
            Row::new(
                f,
                "remote newer",
                &id,
                Some(remote_version),
                "pull is not built yet",
            )
        } else {
            Row::new(
                f,
                "conflict",
                &id,
                Some(remote_version),
                "changed here and in Decisions; nothing sent",
            )
        });
    }
    if remote_version < local_version {
        return Ok(Row::new(
            f,
            "inconsistent",
            &id,
            Some(remote_version),
            "the file records a newer version than Decisions has",
        ));
    }
    if unchanged {
        return Ok(Row::new(f, "unchanged", &id, Some(remote_version), ""));
    }
    if dry {
        return Ok(Row::new(f, "would save", &id, Some(remote_version), ""));
    }
    match save_text(api, space, &id, remote_version, plan.text, MESSAGE).await {
        Ok(saved) => {
            let version = saved
                .document
                .as_ref()
                .map_or(saved.version, |d| d.current_version);
            write_back(
                f,
                &[
                    ("remote_version", version.to_string()),
                    ("remote_sha", plan.sha),
                ],
            )?;
            Ok(Row::new(f, "saved", &id, Some(version), ""))
        }
        Err(Error::Api { error, .. }) if error.status() == Some(409) => {
            // Someone saved between our read and our save: read again, report, overwrite nothing.
            let now = decisions(api)
                .get_document(space, &id, &GetDocumentParams::default())
                .await
                .map_err(scoped)?
                .value;
            let v = now
                .document
                .as_ref()
                .map_or(now.version, |d| d.current_version);
            Ok(Row::new(
                f,
                "conflict",
                &id,
                Some(v),
                "saved in Decisions since the read; nothing sent",
            ))
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::roles_result;
    use crate::roles::{Outcome, Report};
    use crate::ui::Mark;

    fn outcome(name: &str, action: &'static str) -> Outcome {
        Outcome {
            name: name.to_owned(),
            action,
            version: None,
            from: None,
            note: String::new(),
        }
    }

    #[test]
    fn roles_in_the_account_are_a_check_and_roles_kept_local_are_a_warning_with_the_fix() {
        let ok = Report {
            outcomes: vec![
                outcome("security", "written"),
                outcome("python-engineer", "written"),
            ],
            signed_in: true,
            ..Report::default()
        };
        assert_eq!(
            roles_result(&ok),
            (
                Mark::Ok,
                "Roles in your account: security, python-engineer".to_owned()
            )
        );
        let local = Report {
            outcomes: vec![outcome("security", "local")],
            signed_in: false,
            message: Some("Not signed in: run `iohr login`.".to_owned()),
            ..Report::default()
        };
        let (mark, said) = roles_result(&local);
        assert_eq!(mark, Mark::Warn);
        assert_eq!(
            said,
            "Roles kept in atlas/roles/ (security): Not signed in: run `iohr login`."
        );
        let failed = Report {
            outcomes: vec![outcome("security", "failed")],
            signed_in: true,
            message: Some("InOrbit answered 500".to_owned()),
            ..Report::default()
        };
        assert_eq!(
            roles_result(&failed),
            (
                Mark::Warn,
                "Roles not imported (security): InOrbit answered 500".to_owned()
            )
        );
    }
}
