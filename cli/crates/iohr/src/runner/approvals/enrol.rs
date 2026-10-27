//! Enrolling the run key through an ask (platform RFC 0172, amendment 1): `iohr runner
//! run-key` files a `runner_enrollment` ask naming the key, shows its fingerprint here,
//! waits while the person approves it on the phone (the device key's signature is the
//! proof), then signs the possession message naming that ask and enrols. Nobody copies
//! a file between the phone and the machine.
//!
//! The service composes the ask's words from the key and computes the fingerprint
//! itself; this side computes it too, so the terminal shows the same 24 hex characters
//! the phone does without trusting the reply. Asking again while an ask is open gets
//! the same ask back, so a rerun resumes the wait.

use std::time::Duration;

use inorbithr::Method;
use serde_json::{Value, json};

use super::is_approval_id;
use crate::runner::service::{HttpService, ServiceError};
use crate::runner::signing::{
    RunKey, enrollment_possession_message, fingerprint_short, run_key_fingerprint,
};

/// The ask's kind.
pub(crate) const KIND: &str = "runner_enrollment";

/// How long one run of the command waits for the person; a rerun resumes the wait.
pub(crate) const WAIT: Duration = Duration::from_secs(15 * 60);

/// How often the ask is read while waiting.
pub(crate) const POLL: Duration = Duration::from_secs(3);

/// What the enrolment needs from the approvals service.
pub(crate) trait EnrolAsks {
    /// Files (or, while one is open, gets back) the `runner_enrollment` ask.
    async fn file(
        &self,
        public_key: &str,
        label: &str,
        fingerprint: &str,
    ) -> Result<Value, ServiceError>;
    /// The ask as it stands (`Get`).
    async fn get(&self, approval_id: &str) -> Result<Value, ServiceError>;
    /// Enrols with the approved ask and the run key's possession (`EnrollRunner`).
    async fn enroll(
        &self,
        public_key: &str,
        label: &str,
        approval_id: &str,
        possession: &str,
    ) -> Result<Value, ServiceError>;
}

/// The ask this command filed, as the terminal shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Asked {
    pub(crate) approval_id: String,
    pub(crate) fingerprint: String,
    pub(crate) short: String,
}

/// How the wait ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Enrolled: the runner's id.
    Enrolled(String),
    /// The person denied it, or it went stale or was cancelled: the state.
    Closed(String),
    /// Still waiting when the time ran out; a rerun resumes.
    Waiting,
}

/// Files the ask and returns what to show; checks the service names the same
/// fingerprint as this side computed, so a reply cannot show the person another key.
pub(crate) async fn ask(svc: &impl EnrolAsks, key: &RunKey, label: &str) -> Result<Asked, String> {
    let public = key.public_key();
    let fingerprint = run_key_fingerprint(&public)?;
    let a = svc
        .file(&public, label, &fingerprint)
        .await
        .map_err(|e| format!("the enrolment ask could not be filed: {e}"))?;
    let approval_id = str_at(&a, &["approval_id"]);
    if !is_approval_id(&approval_id) {
        return Err("the service answered without an approval id".into());
    }
    let theirs = str_at(&a, &["payload", "runner_enrollment", "fingerprint"]);
    if !theirs.is_empty() && theirs != fingerprint {
        return Err(
            "the service names another fingerprint for this key; nothing was enrolled".into(),
        );
    }
    Ok(Asked {
        approval_id,
        short: fingerprint_short(&fingerprint),
        fingerprint,
    })
}

/// Waits up to `wait` for the person's decision, reading the ask every `poll`; once
/// approved, signs the possession message naming the ask and enrols.
pub(crate) async fn wait_and_enroll(
    svc: &impl EnrolAsks,
    key: &RunKey,
    label: &str,
    asked: &Asked,
    wait: Duration,
    poll: Duration,
) -> Result<Outcome, String> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let a = svc
            .get(&asked.approval_id)
            .await
            .map_err(|e| format!("the enrolment ask could not be read: {e}"))?;
        match str_at(&a, &["status"]).as_str() {
            "approved" => break,
            "pending" | "question" => {}
            other => return Ok(Outcome::Closed(other.to_owned())),
        }
        if tokio::time::Instant::now() + poll > deadline {
            return Ok(Outcome::Waiting);
        }
        tokio::time::sleep(poll).await;
    }
    let public = key.public_key();
    let message =
        enrollment_possession_message(&asked.approval_id, &public, label, &asked.fingerprint);
    let possession = key.sign(message.as_bytes())?;
    let runner = svc
        .enroll(&public, label, &asked.approval_id, &possession)
        .await
        .map_err(|e| format!("the approved enrolment was refused: {e}"))?;
    let id = str_at(&runner, &["runner_id"]);
    if id.is_empty() {
        return Err("the service enrolled the key but named no runner".into());
    }
    Ok(Outcome::Enrolled(id))
}

fn str_at(v: &Value, path: &[&str]) -> String {
    path.iter()
        .try_fold(v, |v, k| v.get(*k))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

impl EnrolAsks for HttpService<'_> {
    async fn file(
        &self,
        public_key: &str,
        label: &str,
        fingerprint: &str,
    ) -> Result<Value, ServiceError> {
        // The service writes the title, summary and meanings itself; these are only
        // what an older service would show.
        let body = json!({
            "account_id": self.account,
            "kind": KIND,
            "title": format!("Enrol runner key: {label}"),
            "summary": format!("Enrol run key {label} ({}).", fingerprint_short(fingerprint)),
            "approve_means": "This run key may run commands you approve for it.",
            "agent": "iohr runner run-key",
            "payload": {"runner_enrollment": {
                "public_key": public_key,
                "label": label,
                "fingerprint": fingerprint,
            }},
        });
        let v: Value = self
            .api
            .send_body(Method::Post, "/v1/approvals", &body)
            .await?
            .json()?;
        Ok(v.get("approval").cloned().unwrap_or(Value::Null))
    }

    async fn get(&self, approval_id: &str) -> Result<Value, ServiceError> {
        if !is_approval_id(approval_id) {
            return Err(ServiceError::Refused("an approval id that is not one"));
        }
        let v: Value = self
            .api
            .get(
                &format!("/v1/approvals/{approval_id}"),
                &[("account_id", self.account.as_str())],
            )
            .await?;
        Ok(v.get("approval").cloned().unwrap_or(Value::Null))
    }

    async fn enroll(
        &self,
        public_key: &str,
        label: &str,
        approval_id: &str,
        possession: &str,
    ) -> Result<Value, ServiceError> {
        let body = json!({
            "account_id": self.account,
            "public_key": public_key,
            "label": label,
            "approval_id": approval_id,
            "possession": possession,
        });
        let v: Value = self
            .api
            .send_body(Method::Post, "/v1/approvals/runners", &body)
            .await?
            .json()?;
        Ok(v.get("runner").cloned().unwrap_or(Value::Null))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    use std::sync::Mutex;

    use aws_lc_rs::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;

    use super::*;

    const ID: &str = "apr_01M5Z1KKYQ16SMN4XFYS5K5RFA";

    /// A service whose ask answers `statuses` in turn (the last one repeats).
    struct Fake {
        statuses: Mutex<Vec<&'static str>>,
        fingerprint: Option<String>,
        filed: Mutex<Vec<Value>>,
        enrolled: Mutex<Vec<(String, String, String, String)>>,
    }

    fn fake(statuses: &[&'static str]) -> Fake {
        Fake {
            statuses: Mutex::new(statuses.iter().rev().copied().collect()),
            fingerprint: None,
            filed: Mutex::default(),
            enrolled: Mutex::default(),
        }
    }

    impl EnrolAsks for Fake {
        async fn file(&self, p: &str, l: &str, f: &str) -> Result<Value, ServiceError> {
            self.filed.lock().unwrap().push(json!([p, l, f]));
            let fp = self.fingerprint.clone().unwrap_or_else(|| f.to_owned());
            Ok(json!({"approval_id": ID, "status": "pending",
                "payload": {"runner_enrollment": {"public_key": p, "label": l, "fingerprint": fp}}}))
        }
        async fn get(&self, id: &str) -> Result<Value, ServiceError> {
            let mut s = self.statuses.lock().unwrap();
            let status = if s.len() > 1 { s.pop().unwrap() } else { s[0] };
            Ok(json!({"approval_id": id, "status": status}))
        }
        async fn enroll(
            &self,
            p: &str,
            l: &str,
            id: &str,
            pos: &str,
        ) -> Result<Value, ServiceError> {
            self.enrolled
                .lock()
                .unwrap()
                .push((p.into(), l.into(), id.into(), pos.into()));
            Ok(json!({"runner_id": "rnr_01M5Z1KKYQ16SMN4XFYS5K5RFB"}))
        }
    }

    fn key() -> RunKey {
        RunKey::generate().unwrap().0
    }

    #[test]
    fn the_fingerprint_and_short_form_match_core() {
        // Core's example (`schemas/approvals/runner-enrollment/examples/minimal.json`):
        // the same fingerprint, and the same short form the phone shows.
        let public = "BNPx7_nAKwNo0cXeKOhW5OWUcpoxIaNINbzkPIZpo880U-2FrrUIkBv3Zr_izrA7Ksek8pUsGV0JDAqos9gyrKU";
        let fp = run_key_fingerprint(public).unwrap();
        assert_eq!(
            fp,
            "e7fda60e9a0b4ff223c257694d8cf1a050684200341d44fbee26dac041bd0774"
        );
        assert_eq!(fingerprint_short(&fp), "e7fd a60e 9a0b 4ff2 23c2 5769");
        assert!(run_key_fingerprint("not base64!").is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn an_approved_ask_enrols_with_a_possession_naming_it() {
        let svc = fake(&["pending", "pending", "approved"]);
        let k = key();
        let asked = ask(&svc, &k, "owner Mac").await.unwrap();
        assert_eq!(asked.approval_id, ID);
        assert_eq!(
            asked.fingerprint,
            run_key_fingerprint(&k.public_key()).unwrap()
        );
        let out = wait_and_enroll(&svc, &k, "owner Mac", &asked, WAIT, POLL)
            .await
            .unwrap();
        assert_eq!(
            out,
            Outcome::Enrolled("rnr_01M5Z1KKYQ16SMN4XFYS5K5RFB".into())
        );
        let e = svc.enrolled.lock().unwrap();
        let (public, label, id, possession) = &e[0];
        assert_eq!((label.as_str(), id.as_str()), ("owner Mac", ID));
        let message = [
            "iohr-runner-enrollment-v1",
            ID,
            public,
            "owner Mac",
            &asked.fingerprint,
        ]
        .join("\n");
        let point = B64URL.decode(public).unwrap();
        let sig = B64URL.decode(possession).unwrap();
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, point)
            .verify(message.as_bytes(), &sig)
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_denied_or_stale_ask_enrols_nothing() {
        for state in ["denied", "stale", "cancelled"] {
            let svc = fake(&["pending", state]);
            let k = key();
            let asked = ask(&svc, &k, "l").await.unwrap();
            let out = wait_and_enroll(&svc, &k, "l", &asked, WAIT, POLL)
                .await
                .unwrap();
            assert_eq!(out, Outcome::Closed(state.into()));
            assert!(svc.enrolled.lock().unwrap().is_empty());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_wait_ends_and_a_rerun_resumes() {
        let svc = fake(&["pending"]);
        let k = key();
        let asked = ask(&svc, &k, "l").await.unwrap();
        let out = wait_and_enroll(&svc, &k, "l", &asked, Duration::from_secs(10), POLL)
            .await
            .unwrap();
        assert_eq!(out, Outcome::Waiting);
        assert!(svc.enrolled.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_service_naming_another_fingerprint_is_refused() {
        let mut svc = fake(&["approved"]);
        svc.fingerprint = Some("0".repeat(64));
        let err = ask(&svc, &key(), "l").await.unwrap_err();
        assert!(err.contains("another fingerprint"), "{err}");
    }
}
