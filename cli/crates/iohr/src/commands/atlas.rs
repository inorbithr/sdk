//! `iohr atlas init|new|sync` and `iohr sync` (RFC 0205): the project's own PRDs, RFCs, ADRs
//! and studies in `atlas/`, and the push of them to its Decisions space.
//!
//! Nothing leaves the machine unless the person runs `sync`. The output names files, ids and
//! versions; it never prints a document's text or a finding's message.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::Env;
use crate::atlas::{self, Kind};
use crate::cli::{AtlasCommand, AtlasKind, Global, SyncArgs};
use crate::commands::decisions::{create_in, decisions, save_text, scoped};
use crate::context::{Api, Ctx, session};
use crate::error::Error;
use crate::output::Out;
use crate::project::read_space_link;
use crate::ui::Ui;
use inorbithr::public::DecisionsGetDocumentParams as GetDocumentParams;

const MESSAGE: &str = "Synced from atlas by iohr sync";

pub(crate) async fn run(g: &Global, env: &Env, cmd: AtlasCommand, out: Out) -> Result<(), Error> {
    match cmd {
        AtlasCommand::Init { dir } => init(dir, out),
        AtlasCommand::New { kind, title, dir } => new(kind, &title, dir, out),
        AtlasCommand::Sync(args) => sync(g, env, &args, out).await,
    }
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
fn project_meta(root: &Path) -> (String, String) {
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

fn init(dir: Option<PathBuf>, out: Out) -> Result<(), Error> {
    let root = start(dir)?;
    if !root.is_dir() {
        return Err(Error::Usage(format!("{} is not a folder", root.display())));
    }
    let (name, template) = project_meta(&root);
    let created_at = super::project::now();
    let written = atlas::ensure(
        &root,
        &atlas::Meta {
            name: &name,
            template: &template,
            lang: None,
            describe: None,
            created_at: &created_at,
        },
    )
    .map_err(Error::Failed)?;
    if out.json {
        Out::print_json(
            &serde_json::json!({ "root": root.display().to_string(), "written": written }),
        );
        return Ok(());
    }
    let ui = Ui::detect(false);
    if written.is_empty() {
        ui.outro("atlas/ is already complete: nothing written");
    } else {
        ui.note("Written", &written.join("\n"));
        ui.outro("atlas/ ready. Documents stay on this machine until you run `iohr sync`.");
    }
    Ok(())
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

pub(crate) async fn sync(g: &Global, env: &Env, args: &SyncArgs, out: Out) -> Result<(), Error> {
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
        .or_else(|| read_space_link(&root).map(|(id, _)| id))
        .ok_or_else(|| {
            Error::Usage(
                "no Decisions space: give `--space ID`, or link the project with `/inorbit:link <space>`"
                    .into(),
            )
        })?;
    let found = atlas::scan(&root);
    let mut lazy = Lazy { g, env, api: None };
    let mut rows = Vec::new();
    for f in &found {
        rows.push(sync_one(&mut lazy, &space, f, args.dry_run).await?);
    }
    let local = atlas::local_only(&root);
    if out.json {
        Out::print_json(&serde_json::json!({
            "space": space,
            "dry_run": args.dry_run,
            "documents": rows,
            "local_only": local,
        }));
        return Ok(());
    }
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
