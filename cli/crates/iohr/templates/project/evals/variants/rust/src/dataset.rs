//! The dataset: JSON Lines, one case per line, loaded strictly.

use crate::json::{self, Json};
use std::collections::HashSet;
use std::fmt;
use std::path::Path;

pub const DEFAULT_DATASET: &str = "datasets/tickets.jsonl";
pub const DEFAULT_BAR: &str = "thresholds.toml";

/// The dataset or the bar is wrong. The run stops and exits 2: nothing was graded.
#[derive(Debug, Clone, PartialEq)]
pub struct SuiteError(pub String);

impl fmt::Display for SuiteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SuiteError {}

pub fn suite_error<T>(message: String) -> Result<T, SuiteError> {
    Err(SuiteError(message))
}

/// One line of the dataset: an id, an input and the expected answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub id: String,
    pub input: String,
    pub expected: String,
}

/// The reason an I/O error carries, without the OS code, so the message reads the same everywhere.
pub fn reason(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "no such file or directory".to_string(),
        std::io::ErrorKind::PermissionDenied => "permission denied".to_string(),
        _ => e.to_string(),
    }
}

pub fn load_cases(path: &Path) -> Result<Vec<Case>, SuiteError> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        SuiteError(format!(
            "cannot read the dataset {}: {}",
            path.display(),
            reason(&e)
        ))
    })?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut cases = Vec::new();
    let mut seen = HashSet::new();
    for (i, line) in text.lines().enumerate() {
        let number = i + 1;
        if line.trim().is_empty() {
            continue;
        }
        let row = match json::parse(line) {
            Ok(Json::Object(row)) => row,
            Ok(_) => return suite_error(format!("{name} line {number}: expected an object")),
            Err(why) => return suite_error(format!("{name} line {number}: not JSON ({why})")),
        };
        let field = |key: &str| -> Result<String, SuiteError> {
            match row.get(key) {
                Some(Json::Str(s)) if !s.trim().is_empty() => Ok(s.clone()),
                _ => Err(SuiteError(format!(
                    "{name} line {number}: `{key}` must be a non-empty string"
                ))),
            }
        };
        let (id, input, expected) = (field("id")?, field("input")?, field("expected")?);
        if !seen.insert(id.clone()) {
            return suite_error(format!("{name} line {number}: the id `{id}` is used twice"));
        }
        cases.push(Case {
            id,
            input,
            expected,
        });
    }
    if cases.is_empty() {
        return suite_error(format!(
            "{name} has no cases: an empty suite passes nothing"
        ));
    }
    Ok(cases)
}
