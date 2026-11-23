//! The board and card types, and reading and writing `board.json`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{Error, check, parse_date};
use crate::schema::Schemas;

/// A board, as `scrum/board/v0` types it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Board {
    pub(crate) board: String,
    pub(crate) updated: String,
    /// Build hours one day holds across every agent working in parallel; the plan flags a day
    /// above it. Unset means one person's day ([`super::view::DAY_HOURS`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) day_hours: Option<f64>,
    pub(crate) lists: Vec<String>,
    pub(crate) cards: Vec<Card>,
}

/// A card, as `scrum/card/v0` types it. The field order is the key order written to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Card {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) list: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) labels: Vec<String>,
    pub(crate) size: String,
    /// A JSON number, so `6` stays `6` on disk.
    pub(crate) hours: serde_json::Number,
    pub(crate) start: String,
    pub(crate) due: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) blocked_by: Vec<String>,
    pub(crate) docs: Docs,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) builds_on: Vec<String>,
    pub(crate) checklist: Vec<Check>,
    pub(crate) acceptance: Vec<String>,
    pub(crate) tests: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) planner: Option<Planner>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) notes: Option<String>,
    /// Fields a project adds to its cards (`x.section`, `x.ideas`); the schema does not look
    /// inside.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub(crate) x: Map<String, Value>,
}

/// Atlas files a card builds from, by kind.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Docs {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) prd: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) rfc: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) adr: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) study: Vec<String>,
}

impl Docs {
    /// Every path, prd first.
    pub(crate) fn all(&self) -> impl Iterator<Item = &String> {
        self.prd
            .iter()
            .chain(&self.rfc)
            .chain(&self.adr)
            .chain(&self.study)
    }
}

/// One checklist item.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Check {
    pub(crate) text: String,
    pub(crate) done: bool,
}

/// The Planner item a card was synced to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Planner {
    pub(crate) plan: String,
    pub(crate) item: String,
}

impl Card {
    /// `hours` as a float, for sums.
    pub(crate) fn h(&self) -> f64 {
        self.hours.as_f64().unwrap_or(0.0)
    }

    /// Not done and not dropped.
    pub(crate) fn open(&self) -> bool {
        self.list != "done" && self.list != "dropped"
    }

    /// Still to build: open and not waiting in review. A review card's hours are spent, so the
    /// plan leaves it out.
    pub(crate) fn to_build(&self) -> bool {
        self.open() && self.list != "review"
    }

    /// Open and past its due date.
    pub(crate) fn late(&self, today: &str) -> bool {
        self.open()
            && matches!((parse_date(&self.due), parse_date(today)), (Some(d), Some(t)) if d < t)
    }
}

impl Board {
    /// The card with this id.
    pub(crate) fn card(&self, id: &str) -> Result<&Card, Error> {
        self.cards
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| Error::Usage(format!("no card {id} on the board")))
    }

    /// The card with this id, to change.
    pub(crate) fn card_mut(&mut self, id: &str) -> Result<&mut Card, Error> {
        self.cards
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| Error::Usage(format!("no card {id} on the board")))
    }

    /// The list a card is in, or `None` when the id is not on the board.
    pub(crate) fn list_of(&self, id: &str) -> Option<&str> {
        self.cards.iter().find(|c| c.id == id).map(|c| c.list.as_str())
    }
}

/// The nearest `atlas/scrum/board.json`, walking up from `start`.
pub(crate) fn find(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|d| d.join(super::BOARD_PATH))
        .find(|p| p.is_file())
}

/// A board read from disk, with the exact bytes it was read from.
#[derive(Debug, Clone)]
pub(crate) struct Loaded {
    pub(crate) path: PathBuf,
    /// The bytes as read, so [`save`] can refuse when the file changed meanwhile.
    pub(crate) raw: Vec<u8>,
    /// The `atlas/` folder holding the board, when it sits at `atlas/scrum/board.json`.
    /// Docs paths are checked against it; a board elsewhere skips that check.
    pub(crate) atlas: Option<PathBuf>,
    pub(crate) board: Board,
}

fn atlas_of(path: &Path) -> Option<PathBuf> {
    let atlas = path.parent()?.parent()?;
    (atlas.file_name()? == "atlas").then(|| atlas.to_path_buf())
}

fn schemas() -> Result<&'static Schemas, String> {
    static S: OnceLock<Result<Schemas, String>> = OnceLock::new();
    S.get_or_init(Schemas::embedded).as_ref().map_err(Clone::clone)
}

/// Reads and checks a board. An invalid board is refused with every problem found.
pub(crate) fn load(path: &Path) -> Result<Loaded, Error> {
    let raw = fs::read(path)
        .map_err(|e| Error::Io(format!("cannot read {}: {e}", path.display())))?;
    let value: Value = serde_json::from_slice(&raw)
        .map_err(|e| Error::Refused(vec![format!("{} is not JSON: {e}", path.display())]))?;
    let atlas = atlas_of(path);
    let problems = check::check(&value, atlas.as_deref());
    if !problems.is_empty() {
        return Err(Error::Refused(problems));
    }
    let board = serde_json::from_value(value)
        .map_err(|e| Error::Refused(vec![format!("board: {e}")]))?;
    Ok(Loaded {
        path: path.to_path_buf(),
        raw,
        atlas,
        board,
    })
}

/// The schema's verdict on a board value, one line per broken field; empty when valid.
pub(crate) fn schema_problems(value: &Value) -> Vec<String> {
    let s = match schemas() {
        Ok(s) => s,
        Err(e) => return vec![e.clone()],
    };
    match s.validate(crate::schema::SCRUM_BOARD, value) {
        Ok(()) => Vec::new(),
        Err(crate::schema::ValidateError::Invalid(all)) => all
            .iter()
            .map(|v| format!("{}: breaks {}", v.path, v.rule))
            .collect(),
        Err(e) => vec![e.to_string()],
    }
}

/// Checks a changed board, then writes it: a temporary file in the same folder, then a rename.
/// `updated` becomes `today`. Refused when the board is invalid, or when the file on disk is no
/// longer the one [`load`] read (another command or person changed it meanwhile).
pub(crate) fn save(loaded: &Loaded, mut board: Board, today: &str) -> Result<(), Error> {
    board.updated = today.to_string();
    let value = serde_json::to_value(&board).map_err(|e| Error::Io(format!("board: {e}")))?;
    let problems = check::check(&value, loaded.atlas.as_deref());
    if !problems.is_empty() {
        return Err(Error::Refused(problems));
    }
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", loaded.path.display()));
    if fs::read(&loaded.path).map_err(io)? != loaded.raw {
        return Err(Error::Refused(vec![
            "board.json changed since it was read; run the command again".into(),
        ]));
    }
    write(&loaded.path, &board)
}

/// The atomic write: a temporary file beside the board, then a rename.
pub(crate) fn write(path: &Path, board: &Board) -> Result<(), Error> {
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", path.display()));
    let text = pretty(board)?;
    let name = path
        .file_name()
        .map_or("board.json".into(), |n| n.to_string_lossy().into_owned());
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    fs::write(&tmp, text).map_err(io)?;
    fs::rename(&tmp, path).map_err(io)
}

/// The board as it is written: pretty JSON and a final newline.
pub(crate) fn pretty(board: &Board) -> Result<String, Error> {
    let mut text =
        serde_json::to_string_pretty(board).map_err(|e| Error::Io(format!("board: {e}")))?;
    text.push('\n');
    Ok(text)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::scrum::LISTS;
    use serde_json::json;

    /// A valid card with this id, list and dependencies.
    pub(crate) fn card(id: &str, list: &str, deps: &[&str]) -> Card {
        serde_json::from_value(json!({
            "id": id, "title": format!("Card {id}"), "list": list,
            "size": "S", "hours": 2, "start": "2026-11-14", "due": "2026-11-15",
            "depends_on": deps, "docs": {},
            "checklist": [{"text": "one", "done": false}, {"text": "two", "done": false}],
            "acceptance": ["it works"], "tests": ["cargo test"]
        }))
        .expect("test card")
    }

    pub(crate) fn board(cards: Vec<Card>) -> Board {
        Board {
            board: "test".into(),
            updated: "2026-11-14".into(),
            day_hours: None,
            lists: LISTS.map(String::from).to_vec(),
            cards,
        }
    }

    fn problems(b: &Board) -> Vec<String> {
        check::check(&serde_json::to_value(b).expect("value"), None)
    }

    #[test]
    fn graph_problems_are_found() {
        let p = problems(&board(vec![card("aa", "ready", &["aa"])]));
        assert_eq!(p, vec!["aa depends on itself"]);

        let p = problems(&board(vec![
            card("aa", "ready", &["bb"]),
            card("bb", "ready", &["cc"]),
            card("cc", "ready", &["aa"]),
        ]));
        assert_eq!(p, vec!["cycle: aa -> bb -> cc -> aa"]);

        let p = problems(&board(vec![card("aa", "ready", &["zz"])]));
        assert_eq!(p, vec!["aa depends on zz, which is not on the board"]);

        let p = problems(&board(vec![card("aa", "ready", &[]), card("aa", "done", &[])]));
        assert_eq!(p, vec!["card id aa is used twice"]);

        let mut late = card("bb", "ready", &[]);
        late.start = "2026-11-15".into();
        let p = problems(&board(vec![card("aa", "ready", &["bb"]), late]));
        assert_eq!(
            p,
            vec!["aa starts 2026-11-14 before bb, which it depends on, starts 2026-11-15"]
        );

        let mut odd = card("aa", "ready", &[]);
        odd.due = "2026-02-30".into();
        assert!(problems(&board(vec![odd]))[0].contains("not a date"), "impossible day");

        let mut bad = card("aa", "ready", &[]);
        bad.list = "someday".into();
        assert!(problems(&board(vec![bad]))[0].starts_with("/cards/0/list"), "schema first");
    }

    #[test]
    fn save_refuses_a_board_changed_since_it_was_read() {
        let tmp = tempfile::Builder::new()
            .prefix("iohr-scrum-save-")
            .tempdir()
            .expect("dir");
        let dir = tmp.path().to_path_buf();
        let path = dir.join("board.json");
        let b = board(vec![card("aa", "ready", &[])]);
        fs::write(&path, serde_json::to_string_pretty(&b).expect("json")).expect("write");

        let first = load(&path).expect("load");
        assert!(first.atlas.is_none(), "a board outside atlas/ skips the docs check");
        let second = load(&path).expect("load");
        save(&first, first.board.clone(), "2026-11-15").expect("first save");
        let err = save(&second, second.board.clone(), "2026-11-16").expect_err("stale");
        assert_eq!(
            err.to_string(),
            "board.json changed since it was read; run the command again"
        );
        assert_eq!(load(&path).expect("load").board.updated, "2026-11-15");
        assert!(!dir.join(".board.json.tmp").exists(), "the temporary file is renamed away");

        let mut broken = load(&path).expect("load");
        broken.board.cards[0].depends_on = vec!["aa".into()];
        let b2 = broken.board.clone();
        assert_eq!(save(&broken, b2, "2026-11-16").expect_err("invalid").code(), 1);
    }

    #[test]
    fn x_fields_survive_a_round_trip() {
        let mut c = card("aa", "ready", &[]);
        c.x.insert("section".into(), json!("fit"));
        c.x.insert("ideas".into(), json!([1, 2]));
        let v = serde_json::to_value(&c).expect("value");
        assert_eq!(v["x"]["section"], "fit");
        let back: Card = serde_json::from_value(v).expect("card");
        assert_eq!(back.x["ideas"], json!([1, 2]));
        assert!(serde_json::to_value(card("bb", "ready", &[])).expect("v").get("x").is_none());
    }
}
