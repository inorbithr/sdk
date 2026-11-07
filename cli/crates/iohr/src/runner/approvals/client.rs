//! The approvals service as one client (platform RFC 0113): list, read, decide, and the two
//! live streams. The runner's poll, the enrolment wait and `iohr asks` all call these, so
//! there is one hand-written approvals client until the SDK's generated one replaces it
//! (RFC 0204, slice 1).
//!
//! Streams are server-sent events through the SDK client: `StreamEvents` with
//! `types=approval.changed` (a frame names an id, a kind and a status, never a title) and
//! `Watch` (the whole ask with its timeline on each change). A stream has no resume: the
//! caller lists again after every reconnect.

use inorbithr::{EventStream, Method, Operation};
use serde_json::{Value, json};

use super::{MAX_PAGES, is_approval_id, text};
use crate::runner::service::{HttpService, ServiceError};

/// The event type the service publishes when an ask moves.
pub(crate) const CHANGED: &str = "approval.changed";

fn id_path(approval_id: &str, tail: &str) -> Result<String, ServiceError> {
    if is_approval_id(approval_id) {
        Ok(format!("/v1/approvals/{approval_id}{tail}"))
    } else {
        Err(ServiceError::Refused("an approval id that is not one"))
    }
}

impl HttpService<'_> {
    /// The asks in `status` (`""` is every state; `open` is pending and question) and
    /// `kind` (`""` is every kind), page after page up to a bound.
    pub(crate) async fn list_approvals(
        &self,
        status: &str,
        kind: &str,
    ) -> Result<Vec<Value>, ServiceError> {
        let mut all = Vec::new();
        let mut token = String::new();
        for _ in 0..MAX_PAGES {
            let mut q: Vec<(&str, &str)> = vec![("account_id", self.account.as_str())];
            if !status.is_empty() {
                q.push(("status", status));
            }
            if !kind.is_empty() {
                q.push(("kind", kind));
            }
            if !token.is_empty() {
                q.push(("page_token", token.as_str()));
            }
            let v: Value = self.api.get("/v1/approvals", &q).await?;
            if let Some(a) = v.get("approvals").and_then(Value::as_array) {
                all.extend(a.iter().cloned());
            }
            token = text(&v, "next_page_token");
            if token.is_empty() {
                break;
            }
        }
        Ok(all)
    }

    /// The ask with its timeline (`Get`).
    pub(crate) async fn get_approval(&self, approval_id: &str) -> Result<Value, ServiceError> {
        let v: Value = self
            .api
            .get(
                &id_path(approval_id, "")?,
                &[("account_id", self.account.as_str())],
            )
            .await?;
        Ok(v.get("approval").cloned().unwrap_or(Value::Null))
    }

    /// Decides an ask without a proof (`Decide`): `deny` or `question`. Approving and
    /// cancelling need a device key's proof (ADR 0215); see [`Self::decide_signed`].
    pub(crate) async fn decide_approval(
        &self,
        approval_id: &str,
        decision: &str,
        note: &str,
    ) -> Result<Value, ServiceError> {
        self.decide_with(approval_id, decision, note, None).await
    }

    /// Decides an ask with a device key's `proof` (`{key_id, signed_at, nonce, signature}`):
    /// the only way to approve (RFC 0113). The service checks the proof; a refusal is an
    /// error carrying its reason.
    pub(crate) async fn decide_signed(
        &self,
        approval_id: &str,
        decision: &str,
        note: &str,
        proof: &Value,
    ) -> Result<Value, ServiceError> {
        self.decide_with(approval_id, decision, note, Some(proof))
            .await
    }

    async fn decide_with(
        &self,
        approval_id: &str,
        decision: &str,
        note: &str,
        proof: Option<&Value>,
    ) -> Result<Value, ServiceError> {
        let mut body = json!({
            "account_id": self.account,
            "decision": decision,
            "note": note,
        });
        if let (Some(proof), Some(map)) = (proof, body.as_object_mut()) {
            map.insert("proof".to_owned(), proof.clone());
        }
        let v: Value = self
            .api
            .send_body(Method::Post, &id_path(approval_id, "/decision")?, &body)
            .await?
            .json()?;
        Ok(v.get("approval").cloned().unwrap_or(Value::Null))
    }

    /// Enrols a person's device key (`EnrollKey`); `body` is
    /// `{public_key, platform, label, device_id, countersign?}`. Returns the key.
    pub(crate) async fn enroll_key(&self, body: &Value) -> Result<Value, ServiceError> {
        let v: Value = self
            .api
            .send_body(Method::Post, "/v1/approvals/keys", body)
            .await?
            .json()?;
        Ok(v.get("key").cloned().unwrap_or(Value::Null))
    }

    /// The caller's device keys (`ListKeys`), without their public keys.
    pub(crate) async fn list_keys(&self) -> Result<Vec<Value>, ServiceError> {
        let v: Value = self.api.get("/v1/approvals/keys", &[]).await?;
        Ok(v.get("keys")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// Revokes a device key (`RevokeKey`).
    pub(crate) async fn revoke_key(&self, key_id: &str) -> Result<(), ServiceError> {
        if !key_id.starts_with("apk_")
            || !key_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(ServiceError::Refused("a key id that is not one"));
        }
        self.api
            .send(
                Method::Delete,
                &format!("/v1/approvals/keys/{key_id}"),
                &[],
                None,
            )
            .await?;
        Ok(())
    }

    /// Every `approval.changed` event of the account, as they happen.
    pub(crate) async fn approval_changes(&self) -> Result<EventStream<Value>, ServiceError> {
        let op = Operation::new(Method::Get, "/v1/events/events")
            .query("types", CHANGED)
            .query("account_id", self.account.as_str());
        Ok(self.api.client().stream(op).await?)
    }

    /// One ask, whole, on each change (`Watch`).
    pub(crate) async fn watch_approval(
        &self,
        approval_id: &str,
    ) -> Result<EventStream<Value>, ServiceError> {
        let op = Operation::new(Method::Get, id_path(approval_id, "/events")?)
            .query("account_id", self.account.as_str());
        Ok(self.api.client().stream(op).await?)
    }
}
