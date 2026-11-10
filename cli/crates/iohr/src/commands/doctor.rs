//! `iohr doctor`: finds the known local problems and says how to fix each (PRD 0208).

use std::fmt::Write as _;

use crate::Env;
use crate::cli::Global;
use crate::context::{self, Ctx};
use crate::error::Error;
use crate::health::{self, Finding, Fixed, Probes};
use crate::output::Out;

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
        Out::raw(text(&findings, &fixed).as_bytes());
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
