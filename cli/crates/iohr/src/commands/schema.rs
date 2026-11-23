//! `iohr schemas`: a project's JSON Schemas tree, kept in files (RFC 0234, RFC 0222). Check it
//! against the layout of RFC 0147, list and show its types, validate a document against one,
//! and make, version and retire them. Offline: it reads and writes only the folder named.

use std::fmt::Write as _;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::cli::{
    SchemaBump, SchemaCheck, SchemaCommand, SchemaDeprecate, SchemaList, SchemaNew, SchemaShow,
    SchemaValidate,
};
use crate::error::Error;
use crate::output::Out;
use crate::schema::{
    BASE, Problem, Report, Schemas, ValidateError, Violation, check_tree, families, file_name,
    json_files, load_tree, read_json,
};
use crate::schema_manage::{self as manage, Manifest, NewArgs, Plan};
use crate::ui::{CheckRow, Listed, Mark, Paint};

/// Runs one `iohr schemas` command.
///
/// # Errors
///
/// [`Error::Usage`] for a bad name or a folder that is not there, [`Error::Findings`] (exit
/// code 1) for problems found, after printing them.
pub(crate) fn run(cmd: SchemaCommand, out: Out) -> Result<(), Error> {
    match cmd {
        SchemaCommand::Check(a) => check(&a, out),
        SchemaCommand::List(a) => list(&a, out),
        SchemaCommand::Show(a) => show(&a, out),
        SchemaCommand::Validate(a) => validate(&a, out),
        SchemaCommand::New(a) => new(&a, out),
        SchemaCommand::Bump(a) => bump(&a, out),
        SchemaCommand::Deprecate(a) => deprecate(&a, out),
    }
}

/// What `--json` prints.
#[derive(Serialize)]
struct Answer<'a> {
    dir: String,
    types: usize,
    examples: usize,
    invalid: usize,
    problems: &'a [Problem],
}

/// Checks the tree in `args.dir`, and the manifest when there is one.
///
/// # Errors
///
/// [`Error::Usage`] when the folder is missing, [`Error::Findings`] (exit code 1) when a
/// schema breaks a rule, after printing every problem.
pub(crate) fn check(args: &SchemaCheck, out: Out) -> Result<(), Error> {
    let report = check_tree(&args.dir).map_err(Error::Usage)?;
    print_check(&args.dir, &report, out)
}

/// Prints a check's result the way `schemas check` does, and fails when it found problems.
fn print_check(dir: &Path, report: &Report, out: Out) -> Result<(), Error> {
    let n = report.problems.len();
    let plural = |n: usize, one: &str, many: &str| {
        if n == 1 {
            one.to_owned()
        } else {
            many.to_owned()
        }
    };
    let checked = format!(
        "{} {}, {} {}, {} {} checked",
        report.types,
        plural(report.types, "schema", "schemas"),
        report.examples,
        plural(report.examples, "example", "examples"),
        report.invalid,
        plural(report.invalid, "invalid case", "invalid cases"),
    );
    let summary = format!("{checked}, {n} {}", plural(n, "problem", "problems"));
    let paint = Paint::stdout(out.json);
    if out.json {
        Out::print_json(&Answer {
            dir: dir.display().to_string(),
            types: report.types,
            examples: report.examples,
            invalid: report.invalid,
            problems: &report.problems,
        });
    } else if paint.color() {
        let rows: Vec<CheckRow> = report
            .problems
            .iter()
            .map(|p| CheckRow {
                mark: Mark::Fail,
                id: p.target.clone(),
                what: p.message.clone(),
                notes: Vec::new(),
            })
            .collect();
        Out::raw(paint.findings(&rows, &checked).as_bytes());
    } else {
        let mut text = String::new();
        for p in &report.problems {
            let _ = writeln!(text, "{}: {}", p.target, p.message);
        }
        Out::raw(text.as_bytes());
    }
    if n > 0 {
        return Err(Error::Findings(summary));
    }
    if !out.json && !paint.color() {
        Out::note(&summary);
    }
    Ok(())
}

/// The manifest of `dir`, or the reason it cannot be read.
fn manifest_of(dir: &Path) -> Result<Option<Manifest>, Error> {
    Manifest::read(dir).map_err(|e| Error::Failed(format!("{}: {e}", manage::FILE)))
}

/// A type found, by the tree or by the CLI's own set.
struct Resolved {
    schemas: Schemas,
    id: String,
    fam: String,
    major: u32,
    /// The majors the project's tree has; empty for a type that ships inside `iohr`.
    majors: Vec<u32>,
}

fn resolve(dir: &Path, spec: &str) -> Result<Resolved, Error> {
    let (fam, want) = manage::parse_spec(spec).map_err(Error::Usage)?;
    let embedded = Schemas::embedded().map_err(Error::Failed)?;
    let tree = families(dir);
    if let Some(majors) = tree.get(&fam) {
        let major = want.unwrap_or_else(|| majors.last().copied().unwrap_or(0));
        if !majors.contains(&major) {
            let have: Vec<String> = majors.iter().map(|n| format!("v{n}")).collect();
            return Err(Error::Usage(format!(
                "{fam} has no v{major}; it has {}",
                have.join(", ")
            )));
        }
        let mut sink = Report::default();
        let loaded = load_tree(dir, &mut sink).map_err(Error::Usage)?;
        let name = format!("{fam}/v{major}");
        let Some(found) = loaded.found.iter().find(|f| f.name == name) else {
            return Err(Error::Usage(format!(
                "{name} does not load; run `iohr schemas check` to see why"
            )));
        };
        return Ok(Resolved {
            id: found.id.clone(),
            schemas: embedded.with(loaded.loaded),
            fam,
            major,
            majors: majors.clone(),
        });
    }
    let major = want.unwrap_or(0);
    let id = format!("{BASE}{fam}/v{major}.json");
    if embedded.get(&id).is_none() {
        return Err(Error::Usage(format!(
            "no type {fam} in {} or among the schemas iohr ships",
            dir.display()
        )));
    }
    Ok(Resolved {
        schemas: embedded,
        id,
        fam,
        major,
        majors: Vec::new(),
    })
}

/// One row of `schemas list`.
#[derive(Serialize)]
struct TypeRow {
    path: String,
    newest: u32,
    majors: Vec<u32>,
    status: Option<String>,
    visibility: Option<String>,
    owner: Option<String>,
    replaced_by: Option<String>,
    examples: usize,
    invalid: usize,
}

/// Lists the types of the tree in `args.dir`.
fn list(args: &SchemaList, out: Out) -> Result<(), Error> {
    if !args.dir.is_dir() {
        return Err(Error::Usage(format!(
            "{}: no such folder",
            args.dir.display()
        )));
    }
    let manifest = manifest_of(&args.dir)?;
    let rows: Vec<TypeRow> = families(&args.dir)
        .into_iter()
        .map(|(path, majors)| {
            let entry = manifest.as_ref().and_then(|m| m.schemas.get(&path));
            let at = args.dir.join(&path);
            TypeRow {
                newest: majors.last().copied().unwrap_or(0),
                examples: json_files(&at.join("examples")).len(),
                invalid: json_files(&at.join("invalid")).len(),
                status: entry.map(|e| e.status.clone()),
                visibility: entry.map(|e| e.visibility.clone()),
                owner: entry.and_then(|e| e.owner.clone()),
                replaced_by: entry.and_then(|e| e.replaced_by.clone()),
                majors,
                path,
            }
        })
        .collect();
    if out.json {
        Out::print_json(&serde_json::json!({
            "dir": args.dir.display().to_string(),
            "manifest": manifest.is_some(),
            "types": rows,
        }));
        return Ok(());
    }
    let table: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.path.clone(),
                format!("v{}", r.newest),
                r.status.clone().unwrap_or_else(|| "unlisted".into()),
                r.examples.to_string(),
                r.invalid.to_string(),
            ]
        })
        .collect();
    Out::list(
        &["TYPE", "NEWEST", "STATUS", "EXAMPLES", "INVALID"],
        &table,
        &Listed::new("type", "types"),
    );
    Ok(())
}

/// Shows one type.
fn show(args: &SchemaShow, out: Out) -> Result<(), Error> {
    let r = resolve(&args.dir, &args.spec)?;
    let in_tree = !r.majors.is_empty();
    let doc = r.schemas.get(&r.id).cloned().unwrap_or(Value::Null);
    let entry = if in_tree {
        manifest_of(&args.dir)?.and_then(|m| m.schemas.get(&r.fam).cloned())
    } else {
        None
    };
    let at = args.dir.join(&r.fam);
    let names = |sub: &str| -> Vec<String> {
        json_files(&at.join(sub))
            .iter()
            .map(|p| file_name(p))
            .collect()
    };
    let (examples, invalid) = if in_tree {
        (names("examples"), names("invalid"))
    } else {
        (Vec::new(), Vec::new())
    };
    let text_of = |k: &str| doc.get(k).and_then(Value::as_str).map(str::to_owned);
    let majors: Vec<String> = r.majors.iter().map(|n| format!("v{n}")).collect();
    if out.json {
        Out::print_json(&serde_json::json!({
            "type": r.fam,
            "major": r.major,
            "id": r.id,
            "source": if in_tree { "project" } else { "embedded" },
            "majors": r.majors,
            "status": entry.as_ref().map(|e| e.status.clone()),
            "visibility": entry.as_ref().map(|e| e.visibility.clone()),
            "owner": entry.as_ref().and_then(|e| e.owner.clone()),
            "replaced_by": entry.as_ref().and_then(|e| e.replaced_by.clone()),
            "examples": examples,
            "invalid": invalid,
            "schema": doc,
        }));
        return Ok(());
    }
    let mut pairs: Vec<(&str, String)> = vec![
        ("type", format!("{}@v{}", r.fam, r.major)),
        ("id", r.id.clone()),
        (
            "source",
            if in_tree {
                format!("{}", args.dir.display())
            } else {
                "shipped inside iohr".to_owned()
            },
        ),
    ];
    if let Some(t) = text_of("title") {
        pairs.push(("title", t));
    }
    if let Some(d) = text_of("description") {
        pairs.push(("description", d));
    }
    if in_tree {
        pairs.push(("majors", majors.join(", ")));
        pairs.push((
            "status",
            entry
                .as_ref()
                .map_or_else(|| "unlisted".to_owned(), |e| e.status.clone()),
        ));
        if let Some(e) = &entry {
            pairs.push(("visibility", e.visibility.clone()));
            if let Some(o) = &e.owner {
                pairs.push(("owner", o.clone()));
            }
            if let Some(t) = &e.replaced_by {
                pairs.push(("replaced by", t.clone()));
            }
        }
        pairs.push(("examples", examples.join(", ")));
        pairs.push(("invalid", invalid.join(", ")));
    }
    Out::pairs(&pairs);
    Ok(())
}

/// Validates one document against a type.
fn validate(args: &SchemaValidate, out: Out) -> Result<(), Error> {
    let r = resolve(&args.dir, &args.spec)?;
    let doc = read_json(&args.file)
        .map_err(|why| Error::Usage(format!("{}: {why}", args.file.display())))?;
    let found: Vec<Violation> = match r.schemas.validate(&r.id, &doc) {
        Ok(()) => Vec::new(),
        Err(ValidateError::Invalid(v)) => v,
        Err(other) => return Err(Error::Failed(other.to_string())),
    };
    let what = format!("{}@v{}", r.fam, r.major);
    if out.json {
        Out::print_json(&serde_json::json!({
            "file": args.file.display().to_string(),
            "type": what,
            "id": r.id,
            "valid": found.is_empty(),
            "violations": found,
        }));
    } else if found.is_empty() {
        Out::note(&format!("{} is a valid {what}", args.file.display()));
    } else {
        let mut text = String::new();
        for v in &found {
            let _ = writeln!(text, "{}: {v}", args.file.display());
        }
        Out::raw(text.as_bytes());
    }
    if found.is_empty() {
        return Ok(());
    }
    Err(Error::Findings(format!(
        "{} does not match {what}: {} {}",
        args.file.display(),
        found.len(),
        if found.len() == 1 {
            "violation"
        } else {
            "violations"
        }
    )))
}

fn new(args: &SchemaNew, out: Out) -> Result<(), Error> {
    let plan = manage::plan_new(
        &args.dir,
        &args.path,
        &NewArgs {
            title: args.title.clone(),
            description: args.description.clone(),
            owner: args.owner.clone(),
            visibility: args.visibility.clone(),
        },
    )
    .map_err(Error::Usage)?;
    finish(&args.dir, &plan, args.dry_run, out)
}

fn bump(args: &SchemaBump, out: Out) -> Result<(), Error> {
    let plan = manage::plan_bump(&args.dir, &args.path).map_err(Error::Usage)?;
    finish(&args.dir, &plan, args.dry_run, out)
}

fn deprecate(args: &SchemaDeprecate, out: Out) -> Result<(), Error> {
    let plan = manage::plan_deprecate(&args.dir, &args.path, args.replaced_by.as_deref())
        .map_err(Error::Usage)?;
    finish(&args.dir, &plan, args.dry_run, out)
}

/// What `--json` prints for a write command.
#[derive(Serialize)]
struct Changed<'a> {
    dir: String,
    dry_run: bool,
    changes: Vec<ChangeRow>,
    notes: &'a [String],
    /// The check that ran after the write; none for a dry run.
    check: Option<CheckAnswer<'a>>,
}

#[derive(Serialize)]
struct ChangeRow {
    action: &'static str,
    path: String,
}

#[derive(Serialize)]
struct CheckAnswer<'a> {
    types: usize,
    examples: usize,
    invalid: usize,
    problems: &'a [Problem],
}

/// Says what the plan does, does it unless `dry_run`, then runs the check on the result, as
/// core's `manage.py` does: the command fails when the folder it leaves does not pass.
fn finish(dir: &Path, plan: &Plan, dry_run: bool, out: Out) -> Result<(), Error> {
    let rows: Vec<ChangeRow> = plan
        .changes
        .iter()
        .map(|c| ChangeRow {
            action: c.action.word(),
            path: dir.join(&c.rel).display().to_string(),
        })
        .collect();
    if !dry_run {
        plan.apply(dir).map_err(Error::Failed)?;
    }
    let report = if dry_run {
        None
    } else {
        Some(check_tree(dir).map_err(Error::Failed)?)
    };
    if out.json {
        Out::print_json(&Changed {
            dir: dir.display().to_string(),
            dry_run,
            changes: rows,
            notes: &plan.notes,
            check: report.as_ref().map(|r| CheckAnswer {
                types: r.types,
                examples: r.examples,
                invalid: r.invalid,
                problems: &r.problems,
            }),
        });
        return match report {
            Some(r) if !r.problems.is_empty() => Err(Error::Findings(format!(
                "the result fails the check: {} problems",
                r.problems.len()
            ))),
            _ => Ok(()),
        };
    }
    let mut text = String::new();
    for r in &rows {
        let _ = writeln!(
            text,
            "{}{} {}",
            if dry_run { "would " } else { "" },
            r.action,
            r.path
        );
    }
    Out::raw(text.as_bytes());
    for n in &plan.notes {
        Out::note(n);
    }
    match report {
        Some(r) => print_check(dir, &r, out),
        None => Ok(()),
    }
}
