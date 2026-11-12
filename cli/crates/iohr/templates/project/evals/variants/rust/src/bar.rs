//! The bar: what the suite must reach to pass, read from thresholds.toml.

use crate::dataset::{SuiteError, reason, suite_error};
use crate::runner::Report;
use std::collections::BTreeMap;
use std::path::Path;

/// What the suite must reach to pass.
#[derive(Debug, Clone, PartialEq)]
pub struct Bar {
    pub min_pass_rate: f64,
    pub max_errors: u32,
    pub scorers: BTreeMap<String, f64>,
}

type Tables = BTreeMap<String, BTreeMap<String, String>>;

fn is_name(s: &str, dots: bool) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || (dots && c == '.'))
}

/// A number as `[+-]?digits(.digits)?`, the only shape the bar uses.
fn is_number(s: &str) -> bool {
    let s = s.strip_prefix(['+', '-']).unwrap_or(s);
    let (whole, frac) = match s.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (s, None),
    };
    let digits = |t: &str| !t.is_empty() && t.chars().all(|c| c.is_ascii_digit());
    digits(whole) && frac.is_none_or(digits)
}

/// Reads the small part of TOML the bar uses: `[table]` lines, `key = number` lines and `#`
/// comments. The standard library has no TOML reader, and the bar needs nothing more, so anything
/// else is refused rather than guessed at.
fn parse_tables(name: &str, text: &str) -> Result<Tables, SuiteError> {
    let mut tables = Tables::new();
    let mut current: Option<String> = None;
    for (i, raw) in text.lines().enumerate() {
        let number = i + 1;
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(table) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            if !is_name(table, true) {
                return suite_error(format!(
                    "{name}: not valid TOML (line {number}: only `[table]` and `key = number` are read here)"
                ));
            }
            if tables.contains_key(table) {
                return suite_error(format!(
                    "{name}: not valid TOML (line {number}: table [{table}] twice)"
                ));
            }
            tables.insert(table.to_string(), BTreeMap::new());
            current = Some(table.to_string());
            continue;
        }
        let pair = line.split_once('=').map(|(k, v)| (k.trim(), v.trim()));
        let (Some((key, value)), Some(table)) = (pair, current.as_ref()) else {
            return suite_error(format!(
                "{name}: not valid TOML (line {number}: only `[table]` and `key = number` are read here)"
            ));
        };
        if !is_name(key, false) || !is_number(value) {
            return suite_error(format!(
                "{name}: not valid TOML (line {number}: only `[table]` and `key = number` are read here)"
            ));
        }
        let entries = tables.entry(table.clone()).or_default();
        if entries.insert(key.to_string(), value.to_string()).is_some() {
            return suite_error(format!(
                "{name}: not valid TOML (line {number}: key {key} twice)"
            ));
        }
    }
    Ok(tables)
}

pub fn load_bar(path: &Path, names: &[&str]) -> Result<Bar, SuiteError> {
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let text = std::fs::read_to_string(path).map_err(|e| {
        SuiteError(format!(
            "cannot read the bar {}: {}",
            path.display(),
            reason(&e)
        ))
    })?;
    let tables = parse_tables(&file, &text)?;
    let Some(bar) = tables.get("bar") else {
        return suite_error(format!("{file}: the [bar] table is missing"));
    };
    let rate = bar.get("min_pass_rate").and_then(|v| v.parse::<f64>().ok());
    let errors = bar.get("max_errors").and_then(|v| v.parse::<i64>().ok());
    let (Some(rate), Some(errors), Some(per)) = (rate, errors, tables.get("bar.scorers")) else {
        return suite_error(format!(
            "{file}: [bar] needs min_pass_rate, max_errors and [bar.scorers]"
        ));
    };
    let range = format!("{file}: every rate is a number from 0 to 1");
    if !(0.0..=1.0).contains(&rate) {
        return suite_error(range);
    }
    let mut scorers = BTreeMap::new();
    for (k, v) in per {
        let f: f64 = v.parse().unwrap_or(f64::NAN);
        if !(0.0..=1.0).contains(&f) {
            return suite_error(range);
        }
        scorers.insert(k.clone(), f);
    }
    let Ok(max_errors) = u32::try_from(errors) else {
        return suite_error(format!("{file}: max_errors cannot be negative"));
    };
    let unguarded: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| !scorers.contains_key(*n))
        .collect();
    let unknown: Vec<&str> = scorers
        .keys()
        .map(String::as_str)
        .filter(|k| !names.contains(k))
        .collect();
    if !unguarded.is_empty() {
        let mut sorted = unguarded;
        sorted.sort_unstable();
        return suite_error(format!(
            "{file}: no bar for the scorer(s) {}; a scorer with no bar guards nothing",
            sorted.join(", ")
        ));
    }
    if !unknown.is_empty() {
        return suite_error(format!(
            "{file}: a bar for {}, which is not a scorer",
            unknown.join(", ")
        ));
    }
    Ok(Bar {
        min_pass_rate: rate,
        max_errors,
        scorers,
    })
}

/// Every line of the bar the report does not reach; empty means the bar is met.
pub fn missed(report: &Report, bar: &Bar) -> Vec<String> {
    let mut out = Vec::new();
    if report.pass_rate() < bar.min_pass_rate {
        out.push(format!(
            "pass rate {:.3} is below {:.3}",
            report.pass_rate(),
            bar.min_pass_rate
        ));
    }
    if report.errors() > bar.max_errors as usize {
        out.push(format!(
            "{} error(s), at most {} allowed",
            report.errors(),
            bar.max_errors
        ));
    }
    for (name, min) in &bar.scorers {
        let mean = report.mean(name);
        if mean < *min {
            out.push(format!("scorer {name}: mean {mean:.3} is below {min:.3}"));
        }
    }
    out
}
