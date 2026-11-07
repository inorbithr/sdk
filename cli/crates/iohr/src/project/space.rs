//! The project's space in InOrbit: what `iohr init` asks for, what it decides, and what it
//! writes on this machine (RFC 0127.1, PRD 0088, RFC 0115).
//!
//! A project is a Decisions space of kind `project` in an account: the person's own, or a team
//! (an org account). The state machine of one sync, every state, transition and failure:
//!
//! ```text
//!            +-- no session / 401 / 403 / no network / 5xx --> Queued (written to the state folder)
//! Start ---> Account chosen --list by slug--> found, kind project ----> Reused  (link written)
//!            |                          |---> found, other kind -----> Failed  (name is taken)
//!            |                          '---> not found --create--> Created (link written)
//!            |                                                '--already_exists (a race) --list again--> Reused
//!            '-- --team names an account the person is not in --> refused before anything is made
//! Queued --`iohr project sync`, signed in--> Account chosen   (the queue file is removed on Reused or Created)
//! ```
//!
//! Invariants: nothing is created twice (the slug is the project's name and the list runs first,
//! so a repeat finds it); the link file holds an id and a name and never a secret; a failure
//! that waiting can cure is queued and said so, never an init that fails.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{SPACE_FILE, read_space_link};

/// The kind a project's space has.
pub const KIND: &str = "project";

/// The environment whose link is the committed one.
pub const PRODUCTION: &str = "production";

/// Who may read the project's documents in the account, as the console's space form offers it
/// (`floorOf` in `space-form.ts`): everyone in the account, or only its owners and admins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Readers {
    /// Everyone in the account (no floor of the space's own).
    #[default]
    Team,
    /// Only the account's owners and admins (the `internal` floor).
    Managers,
}

impl Readers {
    /// The values `--visibility` accepts, in the order a menu offers them.
    pub const NAMES: [&'static str; 2] = ["team", "managers"];

    /// Reads a `--visibility` value.
    ///
    /// # Errors
    ///
    /// The value is none of [`Readers::NAMES`].
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "team" => Ok(Self::Team),
            "managers" => Ok(Self::Managers),
            other => Err(format!(
                "--visibility {other}: use team (everyone in the account) or managers (its owners and admins)"
            )),
        }
    }

    /// The `access_floor` the API takes.
    #[must_use]
    pub fn floor(self) -> &'static str {
        match self {
            Self::Team => "",
            Self::Managers => "internal",
        }
    }

    /// The words the report and the menu use, for an account of this kind.
    #[must_use]
    pub fn said(self, personal: bool) -> &'static str {
        match (self, personal) {
            (Self::Team, true) => "only you",
            (Self::Team, false) => "everyone in the team",
            (Self::Managers, _) => "the account's owners and admins",
        }
    }
}

/// What the person asked for, flags and answers together.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ask {
    /// `--team`: an account's id or slug. `None`: the signed-in account.
    pub team: Option<String>,
    /// `--visibility`.
    pub readers: Readers,
}

/// A space the list returned, reduced to what the decision reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub space_id: String,
    pub slug: String,
    pub name: String,
    pub kind: String,
}

/// What to do about the project's name in the account's spaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// A project space of that name exists: link to it, create nothing.
    Reuse { id: String, name: String },
    /// None exists: create it.
    Create,
    /// A space of another kind holds the slug.
    Taken { kind: String },
}

/// Decides from the account's spaces. The slug is the project's name (`check_name` makes it a
/// valid slug), so a second run finds what the first made.
#[must_use]
pub fn decide(spaces: &[Seen], name: &str) -> Decision {
    match spaces.iter().find(|s| s.slug == name) {
        Some(s) if s.kind == KIND => Decision::Reuse {
            id: s.space_id.clone(),
            name: if s.name.is_empty() {
                name.to_owned()
            } else {
                s.name.clone()
            },
        },
        Some(s) => Decision::Taken {
            kind: if s.kind.is_empty() {
                "space".to_owned()
            } else {
                s.kind.clone()
            },
        },
        None => Decision::Create,
    }
}

/// The link file of an environment: production's is the committed [`SPACE_FILE`]; another
/// environment's (`local`) is `.inorbit/space.<env>`, which `.inorbit/.gitignore` keeps out of
/// the repository, because its id means something on one stack only.
#[must_use]
pub fn link_file(env: &str) -> String {
    if env == PRODUCTION {
        SPACE_FILE.to_owned()
    } else {
        format!("{SPACE_FILE}.{env}")
    }
}

/// `.inorbit/.gitignore`: every environment's link but production's stays on the machine.
pub const GITIGNORE_FILE: &str = ".inorbit/.gitignore";
/// The text of [`GITIGNORE_FILE`].
pub const GITIGNORE_TEXT: &str =
    "# A link to a space on a local stack means nothing on another machine.\nspace.*\n";

/// The text of a link file: `id` and `name` (what `read_space_link` reads) and the account.
#[must_use]
pub fn link_text(env: &str, id: &str, name: &str, account: &str) -> String {
    format!(
        "# The InOrbit space this folder records to on {env} (RFC 0103).\n# Committed, so everyone working here records to the same space. No secrets.\nid = {id:?}\nname = {name:?}\naccount = {account:?}\n"
    )
}

/// Writes the link of `env` under `dir`, and the folder's `.gitignore` when it has none.
///
/// # Errors
///
/// A file cannot be written.
pub fn write_link(
    dir: &Path,
    env: &str,
    id: &str,
    name: &str,
    account: &str,
) -> Result<PathBuf, String> {
    let put = |rel: &str, text: &str| -> Result<PathBuf, String> {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        std::fs::write(&p, text).map_err(|e| format!("cannot write {}: {e}", p.display()))?;
        Ok(p)
    };
    if !dir.join(GITIGNORE_FILE).exists() {
        put(GITIGNORE_FILE, GITIGNORE_TEXT)?;
    }
    put(&link_file(env), &link_text(env, id, name, account))
}

/// The space a folder is linked to on `env`: `(id, name)`. Production reads the committed
/// link; another environment reads its own file and never falls back to production's id,
/// which names a space that stack does not have.
#[must_use]
pub fn read_link_for(dir: &Path, env: &str) -> Option<(String, String)> {
    if env == PRODUCTION {
        return read_space_link(dir);
    }
    let text = std::fs::read_to_string(dir.join(link_file(env))).ok()?;
    let t: toml::Table = text.parse().ok()?;
    let id = t.get("id")?.as_str()?.trim();
    if id.is_empty() {
        return None;
    }
    let name = t
        .get("name")
        .and_then(toml::Value::as_str)
        .unwrap_or_default();
    Some((id.to_owned(), name.to_owned()))
}

/// Puts the space's id into a rendered `inorbit.toml`, where the template left `space_id = ""`.
/// Production's id only: it is the one that is committed.
#[must_use]
pub fn fill_manifest(text: &str, id: &str) -> String {
    text.replace("space_id = \"\"", &format!("space_id = {id:?}"))
}

/// A sync that could not happen yet, kept in the project's state folder so `iohr project sync`
/// does it later with the same answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queued {
    pub name: String,
    pub dir: PathBuf,
    pub env: String,
    pub ask: Ask,
    pub queued_at: String,
    /// Why it was queued, in words with no account data.
    pub why: String,
}

/// Where the queue file of a project lives, under its state folder.
#[must_use]
pub fn queue_path(state: &Path, env: &str) -> PathBuf {
    state.join(format!("space-sync.{env}.toml"))
}

/// Writes the queue file.
///
/// # Errors
///
/// The file cannot be written.
pub fn queue(state: &Path, q: &Queued) -> Result<PathBuf, String> {
    let p = queue_path(state, &q.env);
    std::fs::create_dir_all(state)
        .map_err(|e| format!("cannot create {}: {e}", state.display()))?;
    let text = toml::to_string(q).map_err(|e| format!("cannot write the queue: {e}"))?;
    std::fs::write(&p, text).map_err(|e| format!("cannot write {}: {e}", p.display()))?;
    Ok(p)
}

/// The queued sync of an environment, when there is one.
#[must_use]
pub fn queued(state: &Path, env: &str) -> Option<Queued> {
    toml::from_str(&std::fs::read_to_string(queue_path(state, env)).ok()?).ok()
}

/// Removes the queue file once the space exists; a missing file is fine.
pub fn dequeue(state: &Path, env: &str) {
    let _ = std::fs::remove_file(queue_path(state, env));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(id: &str, slug: &str, kind: &str) -> Seen {
        Seen {
            space_id: id.into(),
            slug: slug.into(),
            name: String::new(),
            kind: kind.into(),
        }
    }

    #[test]
    fn a_repeat_reuses_the_project_and_never_creates_a_second() {
        let all = [
            seen("lab_a", "docs", "space"),
            seen("lab_b", "shop", "project"),
        ];
        assert_eq!(
            decide(&all, "shop"),
            Decision::Reuse {
                id: "lab_b".into(),
                name: "shop".into()
            }
        );
        assert_eq!(decide(&all, "other"), Decision::Create);
        assert_eq!(decide(&[], "shop"), Decision::Create);
    }

    #[test]
    fn a_slug_held_by_another_kind_is_named_not_overwritten() {
        let all = [seen("lab_a", "shop", "department")];
        assert_eq!(
            decide(&all, "shop"),
            Decision::Taken {
                kind: "department".into()
            }
        );
        assert_eq!(
            decide(&[seen("lab_a", "shop", "")], "shop"),
            Decision::Taken {
                kind: "space".into()
            }
        );
    }

    #[test]
    fn visibility_maps_to_the_floor_the_console_form_sends() {
        assert_eq!(Readers::parse("team").unwrap().floor(), "");
        assert_eq!(Readers::parse(" Managers ").unwrap().floor(), "internal");
        assert!(Readers::parse("public").unwrap_err().contains("team"));
        assert_eq!(Readers::Team.said(true), "only you");
        assert_eq!(Readers::Team.said(false), "everyone in the team");
    }

    #[test]
    fn production_is_the_committed_link_and_another_environment_has_its_own_file() {
        assert_eq!(link_file("production"), ".inorbit/space");
        assert_eq!(link_file("local"), ".inorbit/space.local");
        let d = tempfile::tempdir().unwrap();
        write_link(d.path(), "production", "lab_p", "shop", "acc_1").unwrap();
        write_link(d.path(), "local", "lab_l", "shop", "acc_9").unwrap();
        assert_eq!(
            read_link_for(d.path(), "production"),
            Some(("lab_p".into(), "shop".into()))
        );
        assert_eq!(
            read_link_for(d.path(), "local"),
            Some(("lab_l".into(), "shop".into()))
        );
        // The committed reader keeps working on what this wrote.
        assert_eq!(
            read_space_link(d.path()),
            Some(("lab_p".into(), "shop".into()))
        );
        assert_eq!(
            std::fs::read_to_string(d.path().join(GITIGNORE_FILE)).unwrap(),
            GITIGNORE_TEXT
        );
        // Local never reads production's id.
        let e = tempfile::tempdir().unwrap();
        write_link(e.path(), "production", "lab_p", "shop", "acc_1").unwrap();
        assert_eq!(read_link_for(e.path(), "local"), None);
    }

    #[test]
    fn the_manifest_gets_the_id_only_where_the_template_left_it_empty() {
        let t = "[project]\nspace_id = \"\"\nlisting = \"\"\n";
        assert_eq!(
            fill_manifest(t, "lab_1"),
            "[project]\nspace_id = \"lab_1\"\nlisting = \"\"\n"
        );
        let set = "space_id = \"lab_0\"\n";
        assert_eq!(fill_manifest(set, "lab_1"), set);
    }

    #[test]
    fn a_queued_sync_is_kept_with_its_answers_and_removed_when_done() {
        let d = tempfile::tempdir().unwrap();
        let q = Queued {
            name: "shop".into(),
            dir: "/work/shop".into(),
            env: "production".into(),
            ask: Ask {
                team: Some("engineering".into()),
                readers: Readers::Managers,
            },
            queued_at: "2026-11-07T10:00:00Z".into(),
            why: "not signed in".into(),
        };
        let p = queue(d.path(), &q).unwrap();
        assert!(p.is_file());
        assert_eq!(queued(d.path(), "production"), Some(q));
        assert_eq!(queued(d.path(), "local"), None);
        dequeue(d.path(), "production");
        assert_eq!(queued(d.path(), "production"), None);
        dequeue(d.path(), "production");
    }
}
