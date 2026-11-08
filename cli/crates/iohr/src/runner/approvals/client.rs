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
    /// cancelling need a device key's proof, which only the phone holds (ADR 0215).
    pub(crate) async fn decide_approval(
        &self,
        approval_id: &str,
        decision: &str,
        note: &str,
    ) -> Result<Value, ServiceError> {
        let body = json!({
            "account_id": self.account,
            "decision": decision,
            "note": note,
        });
        let v: Value = self
            .api
            .send_body(Method::Post, &id_path(approval_id, "/decision")?, &body)
            .await?
            .json()?;
        Ok(v.get("approval").cloned().unwrap_or(Value::Null))
    }

    /// The account's auto-approve policy and the server's rules (`GetAutoApprovePolicy`): the
    /// answer as the service gives it, `policy` unset when the account never had one.
    pub(crate) async fn auto_approve_policy(&self) -> Result<Value, ServiceError> {
        let v: Value = self
            .api
            .get(
                "/v1/approvals/auto-approve",
                &[("account_id", self.account.as_str())],
            )
            .await?;
        Ok(v)
    }

    /// Asks for an auto-approve policy until `until` (RFC 3339, whole seconds, within 24 hours)
    /// over every kind the never list allows (`RequestAutoApprove`). It creates an
    /// `auto_approve` ask for the phone; the policy is set only when the phone's device key
    /// approves it (ADR 0142), never from here. Answers the ask.
    pub(crate) async fn request_auto_approve(
        &self,
        until: &str,
        device: &str,
    ) -> Result<Value, ServiceError> {
        let body = json!({
            "account_id": self.account,
            "scope": {},
            "until": until,
            "device": device,
        });
        let v: Value = self
            .api
            .send_body(Method::Post, "/v1/approvals/auto-approve", &body)
            .await?
            .json()?;
        Ok(v.get("approval").cloned().unwrap_or(Value::Null))
    }

    /// Turns the auto-approve policy off at once (`RevokeAutoApprovePolicy`): no proof, always
    /// safe, and idempotent. Answers the policy as it is now.
    pub(crate) async fn revoke_auto_approve(&self, note: &str) -> Result<Value, ServiceError> {
        let raw = self
            .api
            .send(
                Method::Delete,
                "/v1/approvals/auto-approve",
                &[("account_id", self.account.as_str()), ("note", note)],
                None,
            )
            .await?;
        let v: Value = raw.json()?;
        Ok(v.get("policy").cloned().unwrap_or(Value::Null))
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
