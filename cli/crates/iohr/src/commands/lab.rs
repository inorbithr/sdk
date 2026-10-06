//! `iohr lab check`: the checks the InOrbit site runs on its own lab documents, on
//! yours (RFC 0035). Offline: it reads only the files it is given.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use iohr_lab::{
    Config, Finding, GENERIC_RULES, Kind, Redaction, check_document, check_folder, parse_rules,
};
use serde::Serialize;

use crate::cli::LabCheck;
use crate::error::Error;
use crate::output::Out;

const DEFAULT_FOLDERS: [&str; 2] = ["docs/rfcs", "docs/studies"];
const DEFAULT_CONFIG: &str = "docs/lab/redaction.json";

/// One finding with the file it is in, as printed and as `--json` gives it.
#[derive(Debug, Serialize)]
struct Located {
    path: String,
    #[serde(flatten)]
    finding: Finding,
}

/// Checks every document under `args.paths` (or the default folders).
///
/// # Errors
///
/// [`Error::Usage`] for a path or config that cannot be read, [`Error::Findings`] when
/// a document has a problem (exit code 1), after printing every one.
pub(crate) fn check(args: &LabCheck, out: Out) -> Result<(), Error> {
    let config = read_config(args.config.as_deref())?;
    let mut redaction = Redaction::new(GENERIC_RULES, &config)
        .map_err(|e| Error::Usage(format!("the lab config: {e}")))?;
    if let Some(path) = &args.strict {
        let rules = parse_rules(&read(path)?)
            .map_err(|e| Error::Usage(format!("{}: not a rules file: {e}", path.display())))?;
        redaction = redaction
            .with_strict(&rules)
            .map_err(|e| Error::Usage(format!("{}: {e}", path.display())))?;
    }

    let paths: Vec<PathBuf> = if args.paths.is_empty() {
        let found: Vec<PathBuf> = DEFAULT_FOLDERS
            .iter()
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .collect();
        if found.is_empty() {
            return Err(Error::Usage(
                "no docs/rfcs or docs/studies here: name the folders to check".into(),
            ));
        }
        found
    } else {
        args.paths.clone()
    };

    let mut findings = Vec::new();
    let mut documents = 0usize;
    for path in &paths {
        if path.is_dir() {
            documents += check_dir(path, &redaction, &mut findings)?;
        } else if path.is_file() {
            let folder = path
                .parent()
                .and_then(Path::file_name)
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let name = file_name(path)?;
            let text = read(path)?;
            documents += 1;
            for f in check_document(&name, &text, Kind::of_folder(folder), &redaction) {
                findings.push(located(path, f));
            }
        } else {
            return Err(Error::Usage(format!(
                "{}: no such file or folder",
                path.display()
            )));
        }
    }

    if out.json {
        Out::print_json(&findings);
    } else {
        let mut text = String::new();
        for l in &findings {
            let line = l.finding.line.map(|n| format!(":{n}")).unwrap_or_default();
            let _ = writeln!(
                text,
                "{}{line} [{}] {}",
                l.path, l.finding.rule, l.finding.message
            );
        }
        Out::raw(text.as_bytes());
    }
    let n = findings.len();
    let summary = format!(
        "{documents} document{} checked, {n} finding{}",
        if documents == 1 { "" } else { "s" },
        if n == 1 { "" } else { "s" }
    );
    if n > 0 {
        return Err(Error::Findings(summary));
    }
    if !out.json {
        Out::note(&summary);
    }
    Ok(())
}

/// The config named, the default one when it exists, or none (the generic rules alone).
fn read_config(named: Option<&Path>) -> Result<Config, Error> {
    let path = match named {
        Some(p) => p.to_path_buf(),
        None if Path::new(DEFAULT_CONFIG).is_file() => PathBuf::from(DEFAULT_CONFIG),
        None => return Ok(Config::default()),
    };
    let text = read(&path)?;
    serde_json::from_str(&text)
        .map_err(|e| Error::Usage(format!("{}: not a lab config: {e}", path.display())))
}

/// Every `*.md` in `dir` but a README, checked; returns how many.
fn check_dir(
    dir: &Path,
    redaction: &Redaction,
    findings: &mut Vec<Located>,
) -> Result<usize, Error> {
    let entries = fs::read_dir(dir).map_err(|e| Error::Usage(format!("{}: {e}", dir.display())))?;
    let mut names: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| Error::Usage(format!("{}: {e}", dir.display())))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // Exactly `.md`, as the site reads them: `X.MD` is not a document.
        let markdown = Path::new(&name).extension().is_some_and(|e| e == "md");
        if markdown && name != "README.md" && entry.path().is_file() {
            names.push(name);
        }
    }
    names.sort();
    let kind = Kind::of_folder(dir.file_name().and_then(|n| n.to_str()).unwrap_or_default());
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let whole = check_folder(&refs);
    for name in &names {
        let path = dir.join(name);
        let text = read(&path)?;
        for f in check_document(name, &text, kind, redaction) {
            findings.push(located(&path, f));
        }
        for f in whole.get(name.as_str()).into_iter().flatten() {
            findings.push(located(&path, f.clone()));
        }
    }
    Ok(names.len())
}

fn located(path: &Path, finding: Finding) -> Located {
    Located {
        path: path.display().to_string(),
        finding,
    }
}

fn file_name(path: &Path) -> Result<String, Error> {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| Error::Usage(format!("{}: not a file", path.display())))
}

fn read(path: &Path) -> Result<String, Error> {
    fs::read_to_string(path).map_err(|e| Error::Usage(format!("{}: {e}", path.display())))
}
