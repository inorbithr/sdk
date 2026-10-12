//! The coordinator service as the runner sees it: the calls that exist today over REST
//! (`RegisterRunner`, `ListRunners`, `RunnerHeartbeat`, `AcceptLease`) and the two the
//! service does not have yet (`fetch_pack`, `report_turn`), which are stubs.
//!
//! The runner's credential is the CLI profile's own session (`context::Api`): no new
//! secret, no new file.

use std::collections::BTreeMap;

use inorbithr::Method;
use serde_json::{Value, json};

use crate::context::Api;

/// Why a service call did not give an answer.
#[derive(Debug)]
pub(crate) enum ServiceError {
    Api(inorbithr::Error),
    /// A stub: the service has no such call yet. Never a fake value.
    NotImplemented(&'static str),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Api(e) => write!(f, "{e}"),
            Self::NotImplemented(what) => write!(f, "not implemented: {what}"),
        }
    }
}

impl From<inorbithr::Error> for ServiceError {
    fn from(e: inorbithr::Error) -> Self {
        Self::Api(e)
    }
}

impl From<ServiceError> for crate::error::Error {
    fn from(e: ServiceError) -> Self {
        match e {
            ServiceError::Api(e) => e.into(),
            ServiceError::NotImplemented(_) => Self::Failed(e.to_string()),
        }
    }
}

/// A provider as the runner reports it (`Provider` in the proto).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderState {
    pub(crate) provider: String,
    /// The sign-in method's name, never a credential.
    pub(crate) method: String,
    pub(crate) signed_in: bool,
}

impl ProviderState {
    fn to_json(&self) -> Value {
        json!({"provider": self.provider, "method": self.method, "signed_in": self.signed_in})
    }
}

/// `RegisterRunnerRequest`.
#[derive(Debug, Clone)]
pub(crate) struct Registration {
    pub(crate) name: String,
    pub(crate) os: String,
    pub(crate) arch: String,
    pub(crate) version: String,
    pub(crate) kinds: Vec<String>,
    pub(crate) capacity: u32,
    pub(crate) always_on: bool,
    pub(crate) providers: Vec<ProviderState>,
    pub(crate) identity_sha256: BTreeMap<String, String>,
}

/// The parts of a `Runner` the loop reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RunnerInfo {
    pub(crate) runner_id: String,
    pub(crate) user: String,
    pub(crate) name: String,
    pub(crate) status: String,
    pub(crate) last_seen_at: String,
    pub(crate) providers: Vec<String>,
}

impl RunnerInfo {
    pub(crate) fn from_json(v: &Value) -> Self {
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        Self {
            runner_id: s("runner_id"),
            user: s("user"),
            name: s("name"),
            status: s("status"),
            last_seen_at: s("last_seen_at"),
            providers: v
                .get("providers")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|p| {
                            let n = p.get("provider").and_then(Value::as_str).unwrap_or("?");
                            if p.get("signed_in").and_then(Value::as_bool).unwrap_or(false) {
                                format!("{n}: signed in")
                            } else {
                                format!("{n}: signed out")
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

/// A `Lease`: the offer of one turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(clippy::struct_field_names, reason = "the proto's names")]
pub(crate) struct Lease {
    pub(crate) lease_id: String,
    pub(crate) job_id: String,
    pub(crate) turn_id: String,
    pub(crate) fencing_token: i64,
}

impl Lease {
    pub(crate) fn from_json(v: &Value) -> Self {
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        // 64-bit integers travel as strings, but a number is read too.
        let fencing_token = match v.get("fencing_token") {
            Some(Value::String(t)) => t.parse().unwrap_or(0),
            Some(n) => n.as_i64().unwrap_or(0),
            None => 0,
        };
        Self {
            lease_id: s("lease_id"),
            job_id: s("job_id"),
            turn_id: s("turn_id"),
            fencing_token,
        }
    }
}

/// `RunnerHeartbeatResponse`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Beat {
    pub(crate) runner: RunnerInfo,
    pub(crate) offered: Vec<Lease>,
}

/// The facts `AcceptLease` carries for the Running gate.
#[derive(Debug, Clone, Copy)]
#[allow(clippy::struct_excessive_bools, reason = "the proto's four facts")]
pub(crate) struct AcceptFacts {
    pub(crate) worktree_fresh: bool,
    pub(crate) worktree_is_main_checkout: bool,
    pub(crate) env_scrubbed: bool,
    pub(crate) caps_set: bool,
}

/// PROPOSAL, not in the service's proto: what a turn needs once the lease is accepted.
/// Written out in the pull request as `GetTurnPack`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(clippy::struct_field_names, reason = "the proposed proto's names")]
pub(crate) struct Pack {
    pub(crate) turn_id: String,
    /// The person whose seat the turn is on; the runner refuses another.
    pub(crate) user: String,
    pub(crate) kind: String,
    pub(crate) pack_sha256: String,
    pub(crate) system_prompt: String,
    /// The new messages (the prompt on stdin).
    pub(crate) prompt: String,
    pub(crate) model: String,
    pub(crate) max_turns: u32,
    pub(crate) allowed_tools: Vec<String>,
    pub(crate) wall_clock_secs: u64,
    pub(crate) resume_session: String,
    pub(crate) resume_runner_id: String,
}

/// PROPOSAL, not in the service's proto: a finished turn, as `ReportTurn` would take it.
#[derive(Debug, Clone)]
#[allow(clippy::struct_field_names, reason = "the proposed proto's names")]
pub(crate) struct TurnReport {
    pub(crate) turn_id: String,
    pub(crate) lease_id: String,
    pub(crate) fencing_token: i64,
    pub(crate) runner_id: String,
    pub(crate) pack_sha256: String,
    /// `replied`, `unshaped`, `cap_hit`, `limit_hit`, `login_expired`, `refused_tool`,
    /// `failed`, `refused_provider`, `refused_user`.
    pub(crate) outcome: String,
    pub(crate) say: String,
    pub(crate) state: String,
    pub(crate) actions: Vec<Value>,
    pub(crate) evidence: Vec<Value>,
    pub(crate) left: Vec<Value>,
    pub(crate) provider: String,
    pub(crate) provider_session_id: String,
    /// Whether the runner resumed (`--resume`) or started fresh.
    pub(crate) resumed: bool,
    pub(crate) provider_version: String,
    pub(crate) num_turns: u64,
    pub(crate) duration_ms: u64,
    /// The provider's own estimate; evidence, never a price.
    pub(crate) total_cost_usd: Option<f64>,
    pub(crate) usage: Value,
    pub(crate) denied_tools: Vec<String>,
    /// Why the runner refused to start, when it did.
    pub(crate) detail: String,
}

impl TurnReport {
    /// The wire shape of the proposed `ReportTurn`; sent when the service has the call.
    #[allow(
        dead_code,
        reason = "the proposed ReportTurn is a stub; its shape is tested"
    )]
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "turn_id": self.turn_id, "lease_id": self.lease_id,
            "fencing_token": self.fencing_token.to_string(), "runner_id": self.runner_id,
            "pack_sha256": self.pack_sha256, "outcome": self.outcome,
            "reply": {"say": self.say, "state": self.state, "actions": self.actions,
                      "evidence": self.evidence, "left": self.left},
            "provider": self.provider, "provider_session_id": self.provider_session_id,
            "resumed": self.resumed, "provider_version": self.provider_version,
            "num_turns": self.num_turns, "duration_ms": self.duration_ms,
            "total_cost_usd": self.total_cost_usd, "usage": self.usage,
            "denied_tools": self.denied_tools, "detail": self.detail,
            // The Ed25519 signature over the reply (RFC 0120): the key is not made yet.
            "signature": "",
        })
    }
}

/// What the loop needs from the service.
pub(crate) trait Service {
    async fn register(&self, r: &Registration) -> Result<RunnerInfo, ServiceError>;
    async fn list_runners(&self, mine: bool) -> Result<Vec<RunnerInfo>, ServiceError>;
    async fn heartbeat(
        &self,
        runner_id: &str,
        providers: &[ProviderState],
    ) -> Result<Beat, ServiceError>;
    async fn accept(&self, lease: &Lease, facts: AcceptFacts) -> Result<(), ServiceError>;
    /// STUB: the service has no way to hand over a turn's pack yet.
    async fn fetch_pack(&self, lease: &Lease) -> Result<Pack, ServiceError>;
    /// STUB: the service has no way to take a turn's result yet.
    async fn report_turn(&self, report: &TurnReport) -> Result<(), ServiceError>;
}

/// The service over REST, called as the profile's own session.
#[derive(Debug)]
pub(crate) struct HttpService<'a> {
    pub(crate) api: &'a Api,
}

impl Service for HttpService<'_> {
    async fn register(&self, r: &Registration) -> Result<RunnerInfo, ServiceError> {
        let body = json!({
            "name": r.name, "os": r.os, "arch": r.arch, "version": r.version,
            "kinds": r.kinds, "capacity": r.capacity, "always_on": r.always_on,
            "providers": r.providers.iter().map(ProviderState::to_json).collect::<Vec<_>>(),
            "identity_sha256": r.identity_sha256,
            // The Ed25519 key is not made yet (see the pull request).
            "public_key": "",
        });
        let v: Value = self
            .api
            .send_body(Method::Post, "/v1/coordinator/runners", &body)
            .await?
            .json()?;
        Ok(RunnerInfo::from_json(
            v.get("runner").unwrap_or(&Value::Null),
        ))
    }

    async fn list_runners(&self, mine: bool) -> Result<Vec<RunnerInfo>, ServiceError> {
        let q: &[(&str, &str)] = if mine { &[("mine", "true")] } else { &[] };
        let v: Value = self.api.get("/v1/coordinator/runners", q).await?;
        Ok(v.get("runners")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(RunnerInfo::from_json).collect())
            .unwrap_or_default())
    }

    async fn heartbeat(
        &self,
        runner_id: &str,
        providers: &[ProviderState],
    ) -> Result<Beat, ServiceError> {
        let body = json!({
            "runner_id": runner_id,
            "providers": providers.iter().map(ProviderState::to_json).collect::<Vec<_>>(),
        });
        let v: Value = self
            .api
            .send_body(
                Method::Post,
                &format!("/v1/coordinator/runners/{runner_id}/heartbeat"),
                &body,
            )
            .await?
            .json()?;
        Ok(Beat {
            runner: RunnerInfo::from_json(v.get("runner").unwrap_or(&Value::Null)),
            offered: v
                .get("offered")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(Lease::from_json).collect())
                .unwrap_or_default(),
        })
    }

    async fn accept(&self, lease: &Lease, facts: AcceptFacts) -> Result<(), ServiceError> {
        let body = json!({
            "lease_id": lease.lease_id,
            "fencing_token": lease.fencing_token.to_string(),
            "worktree_fresh": facts.worktree_fresh,
            "worktree_is_main_checkout": facts.worktree_is_main_checkout,
            "env_scrubbed": facts.env_scrubbed,
            "caps_set": facts.caps_set,
        });
        self.api
            .send_body(
                Method::Post,
                &format!("/v1/coordinator/leases/{}/accept", lease.lease_id),
                &body,
            )
            .await?;
        Ok(())
    }

    #[allow(
        clippy::unused_async_trait_impl,
        reason = "a stub with the real call's shape"
    )]
    async fn fetch_pack(&self, _lease: &Lease) -> Result<Pack, ServiceError> {
        // STUB. The proto has no call that gives a runner the turn's pack (system
        // prompt, prompt, model, caps, tools, resume hint). Proposed: `GetTurnPack`.
        Err(ServiceError::NotImplemented(
            "the service has no GetTurnPack yet (proto proposal in the pull request)",
        ))
    }

    #[allow(
        clippy::unused_async_trait_impl,
        reason = "a stub with the real call's shape"
    )]
    async fn report_turn(&self, _report: &TurnReport) -> Result<(), ServiceError> {
        // STUB. The proto has no call that takes a turn's reply. Proposed: `ReportTurn`.
        Err(ServiceError::NotImplemented(
            "the service has no ReportTurn yet (proto proposal in the pull request)",
        ))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    use super::*;

    #[test]
    fn the_stubs_say_so_and_return_no_value() {
        let r = TurnReport {
            turn_id: "t".into(),
            lease_id: "l".into(),
            fencing_token: 9_007_199_254_740_993,
            runner_id: "r".into(),
            pack_sha256: String::new(),
            outcome: "replied".into(),
            say: "hi".into(),
            state: String::new(),
            actions: vec![],
            evidence: vec![],
            left: vec![],
            provider: "claude".into(),
            provider_session_id: String::new(),
            resumed: false,
            provider_version: String::new(),
            num_turns: 1,
            duration_ms: 2,
            total_cost_usd: None,
            usage: Value::Null,
            denied_tools: vec![],
            detail: String::new(),
        };
        let j = r.to_json();
        // 64-bit integers travel as strings.
        assert_eq!(j["fencing_token"], "9007199254740993");
        assert_eq!(j["reply"]["say"], "hi");
        assert_eq!(j["signature"], "");
    }

    #[test]
    fn a_lease_reads_its_fencing_token_as_string_or_number() {
        let a = Lease::from_json(&json!({"lease_id": "l", "fencing_token": "12"}));
        let b = Lease::from_json(&json!({"lease_id": "l", "fencing_token": 12}));
        assert_eq!((a.fencing_token, b.fencing_token), (12, 12));
    }
}
