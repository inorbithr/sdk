//! Approving from a Mac (RFC 0113, ADR 0215): `iohr asks key enroll|show|remove` and the
//! signed path of `iohr asks approve`. The logic (tier, proof, enrolment body) is in
//! `crate::mac_sign`; this is the terminal around it.
//!
//! What is printed before Touch ID is the ask's own title, kind and payload hash, cleaned of
//! control characters and cut, so the person sees what the prompt will name. Nothing of an
//! ask's text goes to a log or an error.

use serde_json::{Value, json};

use super::{HttpService, Out, Session, clean, headline, scoped, str_of};
use crate::Env;
use crate::cli::{AsksKeyCommand, Global};
use crate::context::{Ctx, account, session};
use crate::error::Error;
use crate::mac_sign::store::{self, Enrolled};
use crate::mac_sign::{self, Route, ToSign};
use crate::runner::approvals::proof_from_json;
use crate::ui::Paint;

/// How an approve attempt went.
pub(super) enum Attempt {
    /// This Mac signed and the service took it: the ask, as it is now.
    Signed(Value),
    /// The phone decides this one; the line says why (or `None`: this Mac has no key yet).
    Phone(Option<String>),
}

/// Tries to approve `ask` from this machine: the tier first, then the key, then Touch ID.
pub(super) async fn attempt(
    g: &Global,
    svc: &HttpService<'_>,
    ask: &Value,
) -> Result<Attempt, Error> {
    if let Route::Phone(why) = mac_sign::route(ask, mac_sign::can_sign_here()) {
        return Ok(Attempt::Phone(Some(why)));
    }
    let ctx = Ctx::load(g)?;
    let dir = store::dir(&ctx.dir);
    let Some(enrolled) = store::load(&dir).map_err(Error::Failed)? else {
        return Ok(Attempt::Phone(None));
    };
    let signer = mac_sign::this_machine(&dir).map_err(Error::Failed)?;
    if signer.public_key().map_err(Error::Failed)? != enrolled.public_key {
        return Err(Error::Failed(
            "the key on this Mac is not the one that was enrolled; run `iohr asks key remove`, then `iohr asks key enroll`".into(),
        ));
    }
    let id = str_of(ask, "approval_id");
    let hash = str_of(ask, "payload_sha256");
    if hash.is_empty() {
        return Err(Error::Failed(
            "the ask carries no payload hash, so there is nothing to sign; approve on your phone"
                .into(),
        ));
    }
    let title = headline(&clean(str_of(ask, "title")), 120);
    let p = Paint::stdout(false);
    Out::note(&format!(
        "Approving {} {}: {}",
        str_of(ask, "kind"),
        id,
        p.name(&title)
    ));
    Out::note(&format!(
        "It signs payload {hash}. Touch ID to approve; Ctrl-C to stop."
    ));
    let proof = mac_sign::sign_decision(
        signer.as_ref(),
        &enrolled.key_id,
        &ToSign {
            approval_id: id,
            payload_sha256: hash,
            decision: "approve",
            note: "",
        },
        &mac_sign::touch_reason(&title, "approve", hash),
        now(),
        &mac_sign::fresh_nonce().map_err(Error::Failed)?,
    )
    .map_err(|e| {
        Error::Failed(format!(
            "{e}; nothing changed (approve on your phone, or try again)"
        ))
    })?;
    let approval = svc
        .decide_signed(id, "approve", "", &proof)
        .await
        .map_err(scoped)?;
    Ok(Attempt::Signed(approval))
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// `iohr asks key ...`.
pub(super) async fn run(g: &Global, env: &Env, cmd: AsksKeyCommand, out: Out) -> Result<(), Error> {
    let ctx = Ctx::load(g)?;
    let dir = store::dir(&ctx.dir);
    match cmd {
        AsksKeyCommand::Enroll { label, proof } => {
            if !mac_sign::can_sign_here() {
                return Err(Error::Failed(
                    "only a Mac signs asks; on this machine approve on your phone".into(),
                ));
            }
            let s = session(g, env).await?;
            enroll(&s, &dir, label, proof.as_deref(), out).await
        }
        AsksKeyCommand::Show => {
            let Some(e) = store::load(&dir).map_err(Error::Failed)? else {
                return Err(Error::Failed(
                    "this machine has no approval key (`iohr asks key enroll`)".into(),
                ));
            };
            let s = session(g, env).await?;
            let svc = HttpService::new(&s.api, account(&s, None)?, None);
            let keys = svc.list_keys().await.map_err(scoped)?;
            let found = keys.iter().find(|k| str_of(k, "key_id") == e.key_id);
            let field = |k: &str| found.map_or("", |f| str_of(f, k)).to_owned();
            if out.json {
                Out::print_json(&json!({
                    "key_id": e.key_id, "label": e.label, "public_key": e.public_key,
                    "known_to_service": found.is_some(),
                    "active_at": field("active_at"), "revoked_at": field("revoked_at"),
                    "last_used_at": field("last_used_at"),
                }));
            } else {
                let known = if found.is_some() {
                    "known"
                } else {
                    "not found (removed elsewhere?)"
                };
                Out::pairs(&[
                    ("key_id", e.key_id.clone()),
                    ("label", e.label.clone()),
                    ("public_key", e.public_key.clone()),
                    ("service", known.to_owned()),
                    ("active_at", field("active_at")),
                    ("revoked_at", field("revoked_at")),
                    ("last_used_at", field("last_used_at")),
                ]);
            }
            Ok(())
        }
        AsksKeyCommand::Remove => {
            let enrolled = store::load(&dir).map_err(Error::Failed)?;
            if let Some(e) = &enrolled {
                let s = session(g, env).await?;
                let svc = HttpService::new(&s.api, account(&s, None)?, None);
                svc.revoke_key(&e.key_id).await.map_err(scoped)?;
            }
            store::remove(&dir).map_err(Error::Failed)?;
            if out.json {
                Out::print_json(&json!({
                    "removed": true, "key_id": enrolled.as_ref().map(|e| &e.key_id),
                }));
            } else {
                Out::note("This Mac's approval key is revoked and deleted.");
            }
            Ok(())
        }
    }
}

async fn enroll(
    s: &Session,
    dir: &std::path::Path,
    label: Option<String>,
    proof: Option<&std::path::Path>,
    out: Out,
) -> Result<(), Error> {
    if let Some(e) = store::load(dir).map_err(Error::Failed)? {
        Out::note(&format!(
            "This Mac is already enrolled as {} (`iohr asks key show`).",
            e.key_id
        ));
        return Ok(());
    }
    let label = label.map_or_else(
        || format!("{} (iohr)", crate::commands::runner::host_name()),
        |l| l.trim().to_owned(),
    );
    if label.is_empty() || label.chars().count() > 100 {
        return Err(Error::Usage("--label: 1 to 100 characters".into()));
    }
    let countersign = match proof {
        None => None,
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| {
                Error::Failed(format!("cannot read {}: {}", path.display(), e.kind()))
            })?;
            let v: Value = serde_json::from_str(&text)
                .map_err(|_| Error::Failed(format!("{} is not JSON", path.display())))?;
            Some(proof_from_json(&v).map_err(Error::Failed)?)
        }
    };
    let signer = mac_sign::this_machine(dir).map_err(Error::Failed)?;
    let public = signer.ensure().map_err(Error::Failed)?;
    let body = mac_sign::enroll_body(
        &public,
        &label,
        &crate::commands::runner::host_name(),
        countersign.as_ref(),
    );
    let svc = HttpService::new(&s.api, account(s, None)?, None);
    let key = svc.enroll_key(&body).await.map_err(scoped)?;
    let key_id = str_of(&key, "key_id").to_owned();
    if key_id.is_empty() {
        return Err(Error::Failed("the service enrolled no key".into()));
    }
    store::save(
        dir,
        &Enrolled {
            key_id: key_id.clone(),
            public_key: public.clone(),
            label: label.clone(),
        },
    )
    .map_err(Error::Failed)?;
    let active_at = str_of(&key, "active_at").to_owned();
    let countered = countersign.is_some();
    if out.json {
        Out::print_json(&json!({
            "key_id": key_id, "label": label, "public_key": public,
            "active_at": active_at, "countersigned": countered,
        }));
    } else {
        Out::pairs(&[
            ("key_id", key_id),
            ("label", label),
            ("active_at", active_at),
            (
                "countersigned",
                if countered { "yes" } else { "no" }.to_owned(),
            ),
        ]);
        if !countered {
            Out::note(
                "Not countersigned by your phone: the key is active at the time above (at once if it is your first key).",
            );
        }
    }
    Ok(())
}
