//! `iohr doctor`: finds the known local problems and says how to fix each (PRD 0208).

use std::fmt::Write as _;

use crate::Env;
use crate::cli::Global;
use crate::context::{self, Ctx};
use crate::error::Error;
use crate::health::{self, Finding, Fixed, Probes};
use crate::output::Out;
use crate::ui::{CheckRow, Mark, Paint};

pub(crate) async fn run(g: &Global, env: &Env, fix: bool, out: Out) -> Result<(), Error> {
    let dir = context::config_dir(g)?;
    let mut probes = health::probe_local(&dir);
    probes.signed_in = signed_in(g, env).await;
    let mut findings = health::evaluate(&probes);
    // `--fix` is a yes for this run; `[doctor] auto_fix = true` is a standing one.
    let auto = fix || probes.config.is_ok() && Ctx::load(g)?.config.doctor.auto_fix_on();
    let fixed = health::apply_safe(&findings, auto, health::run_safe);
    if !fixed.is_empty() {
        // What a fix changed is read again, so the table says what is true now.
        let again = Probes {
            signed_in: probes.signed_in,
            ..health::probe_local(&dir)
        };
        findings = health::evaluate(&again);
    }
    if out.json {
        Out::print_json(&serde_json::json!({
            "ok": findings.iter().all(|f| !f.failing()),
            "findings": findings,
            "fixed": fixed.iter().map(|(id, r)| serde_json::json!({
                "id": id, "result": if *r == Fixed::Yes { "fixed" } else { "still_failing" },
            })).collect::<Vec<_>>(),
        }));
    } else {
        let paint = Paint::stdout(out.json);
        let shown = if paint.color() {
            paint.checks(&rows(&findings, &fixed))
        } else {
            text(&findings, &fixed)
        };
        Out::raw(shown.as_bytes());
    }
    if findings.iter().any(Finding::failing) {
        return Err(Error::Findings(format!(
            "{} problem(s) found",
            findings.iter().filter(|f| f.failing()).count()
        )));
    }
    Ok(())
}

/// Whether a session can be made without a prompt: `None` when the store would ask or the
/// state cannot be told.
async fn signed_in(g: &Global, env: &Env) -> Option<bool> {
    match context::quiet_session(g, env).await {
        Ok(_) => Some(true),
        Err(Error::NotSignedIn(_)) => Some(false),
        Err(_) => None,
    }
}

/// The findings as rows for a person: a failing check gets its fix under it, a check the safe
/// fix mended says so. The same facts as [`text`], which scripts read.
fn rows(findings: &[Finding], fixed: &[(&'static str, Fixed)]) -> Vec<CheckRow> {
    findings
        .iter()
        .map(|f| {
            let mut notes = Vec::new();
            if f.failing() {
                if fixed.contains(&(f.id, Fixed::Still)) {
                    notes.push(("tried", "the safe fix; still failing".to_owned()));
                } else {
                    if let Some(o) = &f.offer {
                        notes.push(("fix", o.clone()));
                    }
                    if f.safe.is_some() {
                        let label = if notes.is_empty() { "fix" } else { "or" };
                        notes.push((label, "iohr doctor --fix".to_owned()));
                    }
                }
            } else if fixed.contains(&(f.id, Fixed::Yes)) {
                notes.push(("fixed", "by the safe fix, just now".to_owned()));
            }
            CheckRow {
                mark: if f.failing() { Mark::Fail } else { Mark::Ok },
                id: f.id.to_owned(),
                what: f.what.clone(),
                notes,
            }
        })
        .collect()
}

fn text(findings: &[Finding], fixed: &[(&'static str, Fixed)]) -> String {
    let mut s = String::new();
    for f in findings {
        let mark = if f.failing() { "FAIL" } else { "ok  " };
        let _ = writeln!(s, "{mark}  {:<7} {}", f.id, f.what);
        if f.failing() {
            if fixed.contains(&(f.id, Fixed::Still)) {
                s.push_str("      tried the safe fix; still failing\n");
            } else {
                if f.safe.is_some() {
                    s.push_str(
                        "      `iohr doctor --fix` fixes it (or `[doctor] auto_fix = true`)\n",
                    );
                }
                if let Some(o) = &f.offer {
                    let _ = writeln!(s, "      fix: {o}");
                }
            }
        }
    }
    for (id, r) in fixed {
        if *r == Fixed::Yes {
            let _ = writeln!(s, "fixed {id}");
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::{Finding, Fixed, Mark, rows};
    use crate::health::SafeFix;

    fn finding(id: &'static str, status: &'static str, offer: Option<&str>, safe: bool) -> Finding {
        Finding {
            id,
            status,
            what: format!("{id} is {status}"),
            offer: offer.map(str::to_owned),
            safe: safe.then_some(SafeFix::StartBridge),
        }
    }

    #[test]
    fn a_row_carries_the_fix_the_safe_fix_or_what_the_safe_fix_did() {
        let found = [
            finding("auth", "signed_out", Some("iohr login"), false),
            finding("bridge", "down", Some("open the app"), true),
            finding("disk", "low", None, true),
            finding("config", "ok", None, false),
        ];
        let r = rows(&found, &[("disk", Fixed::Still), ("config", Fixed::Yes)]);
        assert_eq!(r[0].mark, Mark::Fail);
        assert_eq!(r[0].notes, vec![("fix", "iohr login".to_owned())]);
        assert_eq!(
            r[1].notes,
            vec![
                ("fix", "open the app".to_owned()),
                ("or", "iohr doctor --fix".to_owned())
            ]
        );
        assert_eq!(
            r[2].notes,
            vec![("tried", "the safe fix; still failing".to_owned())]
        );
        assert_eq!(r[3].mark, Mark::Ok);
        assert_eq!(
            r[3].notes,
            vec![("fixed", "by the safe fix, just now".to_owned())]
        );
    }
}
