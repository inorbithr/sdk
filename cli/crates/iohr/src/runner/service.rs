//! The coordinator service as the runner sees it, over REST (core PR #879's proto):
//! `CreateRunnerChallenge`, `RegisterRunner`, `ListRunners`, `RunnerHeartbeat`,
//! `AcceptLease`, `GetTurnPack`, `LeaseHeartbeat` and `ReportTurn`.
//!
//! The runner's credential is the CLI profile's own session (`context::Api`): no new
//! secret, no new file.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use inorbithr::Method;
use serde_json::{Value, json};

use super::signing::Signer;
use crate::context::Api;

/// Why a service call did not give an answer.
#[derive(Debug)]
pub(crate) enum ServiceError {
    Api(inorbithr::Error),
    /// Not possible here: for example a report with no signing key. Never a fake value.
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
    /// The runner's Ed25519 public key: 32 bytes in standard base64 with padding.
    pub(crate) public_key: String,
    /// From `CreateRunnerChallenge`, used once.
    pub(crate) challenge: String,
    /// Ed25519 over `RegisterRunner\n<challenge>\n<public_key>`.
    pub(crate) signature: Vec<u8>,
}

/// `CreateRunnerChallengeResponse`: what a registration is signed over, and the salt the
/// provider identity lines are hashed with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Challenge {
    pub(crate) challenge: String,
    pub(crate) identity_salt: String,
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
    /// How often to call `RunnerHeartbeat` and `LeaseHeartbeat`; 0 when not sent.
    pub(crate) heartbeat_interval_seconds: u64,
}

/// `LeaseHeartbeatResponse`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LeaseBeat {
    /// The turn is past its wall-clock cap plus 60 s: stop it.
    pub(crate) stop: bool,
    pub(crate) stop_reason: String,
}

/// A number that travels as a string (64-bit) or a number.
fn num(v: &Value, k: &str) -> u64 {
    match v.get(k) {
        Some(Value::String(t)) => t.parse().unwrap_or(0),
        Some(n) => n.as_u64().unwrap_or(0),
        None => 0,
    }
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

/// `TurnPack`: what a turn needs once the lease is accepted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(clippy::struct_field_names, reason = "the proto's names")]
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
    pub(crate) thread_id: String,
    pub(crate) job_id: String,
    pub(crate) attempt: u32,
    pub(crate) lease_id: String,
    pub(crate) fencing_token: i64,
    /// The pack's budget in tokens (characters over four).
    pub(crate) token_budget: u32,
    /// The JSON the final message must be.
    pub(crate) reply_shape: String,
}

impl Pack {
    pub(crate) fn from_json(v: &Value) -> Self {
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        Self {
            turn_id: s("turn_id"),
            user: s("user"),
            kind: s("kind"),
            pack_sha256: s("pack_sha256"),
            system_prompt: s("system_prompt"),
            prompt: s("prompt"),
            model: s("model"),
            max_turns: u32::try_from(num(v, "max_turns")).unwrap_or(u32::MAX),
            allowed_tools: v
                .get("allowed_tools")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            wall_clock_secs: num(v, "wall_clock_secs"),
            resume_session: s("resume_session"),
            resume_runner_id: s("resume_runner_id"),
            thread_id: s("thread_id"),
            job_id: s("job_id"),
            attempt: u32::try_from(num(v, "attempt")).unwrap_or(u32::MAX),
            lease_id: s("lease_id"),
            fencing_token: i64::try_from(num(v, "fencing_token")).unwrap_or(0),
            token_budget: u32::try_from(num(v, "token_budget")).unwrap_or(u32::MAX),
            reply_shape: s("reply_shape"),
        }
    }
}

/// `ProposedAction`: a kind and its payload as JSON text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Action {
    pub(crate) kind: String,
    pub(crate) payload_json: String,
}

/// `TurnUsage`: counts by class.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Usage {
    pub(crate) input: u64,
    pub(crate) output: u64,
    pub(crate) cache_read: u64,
    pub(crate) cache_write: u64,
}

/// A finished turn, as `ReportTurn` takes it.
#[derive(Debug, Clone)]
#[allow(clippy::struct_field_names, reason = "the proto's names")]
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
    pub(crate) actions: Vec<Action>,
    pub(crate) evidence: Vec<String>,
    pub(crate) left: Vec<String>,
    pub(crate) provider: String,
    pub(crate) provider_session_id: String,
    /// Whether the runner resumed (`--resume`) or started fresh.
    pub(crate) resumed: bool,
    pub(crate) provider_version: String,
    pub(crate) num_turns: u64,
    pub(crate) duration_ms: u64,
    /// The provider's own estimate; evidence, never a price.
    pub(crate) total_cost_usd: Option<f64>,
    pub(crate) usage: Usage,
    pub(crate) denied_tools: Vec<String>,
    /// Why the runner refused to start, when it did.
    pub(crate) detail: String,
    /// The turn's attempt, as its pack said.
    pub(crate) attempt: u32,
    /// For `limit_hit`: when the plan window resets (RFC 3339), if the provider said.
    pub(crate) resets_at: String,
}

impl TurnReport {
    /// The same for every retry of one report: the lease with its fencing token, and the
    /// turn's attempt (the service is idempotent on `(turn_id, attempt)`).
    pub(crate) fn idempotency_key(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.lease_id, self.fencing_token, self.turn_id, self.attempt
        )
    }

    /// The wire shape of `ReportTurn`, carrying `signature` (64 bytes, base64).
    pub(crate) fn to_json(&self, signature: &[u8]) -> Value {
        json!({
            "turn_id": self.turn_id, "lease_id": self.lease_id,
            "fencing_token": self.fencing_token.to_string(), "runner_id": self.runner_id,
            "pack_sha256": self.pack_sha256, "outcome": self.outcome,
            "reply": {
                "say": self.say, "state": self.state,
                "actions": self.actions.iter()
                    .map(|a| json!({"kind": a.kind, "payload_json": a.payload_json}))
                    .collect::<Vec<_>>(),
                "evidence": self.evidence, "left": self.left,
            },
            "provider": self.provider, "provider_session_id": self.provider_session_id,
            "resumed": self.resumed, "provider_version": self.provider_version,
            "num_turns": self.num_turns, "duration_ms": self.duration_ms.to_string(),
            "total_cost_usd": self.total_cost_usd,
            "usage": {
                "input": self.usage.input.to_string(),
                "output": self.usage.output.to_string(),
                "cache_read": self.usage.cache_read.to_string(),
                "cache_write": self.usage.cache_write.to_string(),
            },
            "denied_tools": self.denied_tools, "detail": self.detail,
            "attempt": self.attempt, "resets_at": self.resets_at,
            "signature": B64.encode(signature),
        })
    }
}

/// What the loop needs from the service.
pub(crate) trait Service {
    /// Whether the service can hand over a turn's pack. While it cannot, the loop does
    /// not accept a lease: it would only abandon it.
    fn turn_pack_available(&self) -> bool;
    async fn challenge(&self) -> Result<Challenge, ServiceError>;
    async fn register(&self, r: &Registration) -> Result<RunnerInfo, ServiceError>;
    async fn list_runners(&self, mine: bool) -> Result<Vec<RunnerInfo>, ServiceError>;
    async fn heartbeat(
        &self,
        runner_id: &str,
        providers: &[ProviderState],
    ) -> Result<Beat, ServiceError>;
    async fn accept(&self, lease: &Lease, facts: AcceptFacts) -> Result<(), ServiceError>;
    async fn fetch_pack(&self, lease: &Lease) -> Result<Pack, ServiceError>;
    /// The lease is alive; says whether the turn must stop.
    async fn lease_heartbeat(&self, lease: &Lease) -> Result<LeaseBeat, ServiceError>;
    /// Delivers a finished turn, signed; the same report again is answered as the first
    /// was (idempotent on the turn and its attempt).
    async fn report_turn(&self, report: &TurnReport) -> Result<(), ServiceError>;
}

/// The service over REST, called as the profile's own session.
#[derive(Debug)]
pub(crate) struct HttpService<'a> {
    pub(crate) api: &'a Api,
    /// The runner's key, to sign reports with; absent for commands that report nothing.
    pub(crate) signer: Option<Signer>,
}

impl Service for HttpService<'_> {
    fn turn_pack_available(&self) -> bool {
        true
    }

    async fn challenge(&self) -> Result<Challenge, ServiceError> {
        let v: Value = self
            .api
            .send_body(
                Method::Post,
                "/v1/coordinator/runners/challenge",
                &json!({}),
            )
            .await?
            .json()?;
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        Ok(Challenge {
            challenge: s("challenge"),
            identity_salt: s("identity_salt"),
        })
    }

    async fn register(&self, r: &Registration) -> Result<RunnerInfo, ServiceError> {
        let body = json!({
            "name": r.name, "os": r.os, "arch": r.arch, "version": r.version,
            "kinds": r.kinds, "capacity": r.capacity, "always_on": r.always_on,
            "providers": r.providers.iter().map(ProviderState::to_json).collect::<Vec<_>>(),
            "identity_sha256": r.identity_sha256,
            "public_key": r.public_key,
            "challenge": r.challenge,
            "signature": B64.encode(&r.signature),
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
            heartbeat_interval_seconds: num(&v, "heartbeat_interval_seconds"),
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

    async fn fetch_pack(&self, lease: &Lease) -> Result<Pack, ServiceError> {
        let token = lease.fencing_token.to_string();
        let v: Value = self
            .api
            .get(
                &format!("/v1/coordinator/leases/{}/pack", lease.lease_id),
                &[("fencing_token", token.as_str())],
            )
            .await?;
        let pack = v.get("pack").unwrap_or(&Value::Null);
        if pack.is_null() {
            return Err(ServiceError::NotImplemented("the service sent no pack"));
        }
        Ok(Pack::from_json(pack))
    }

    async fn lease_heartbeat(&self, lease: &Lease) -> Result<LeaseBeat, ServiceError> {
        // The provider's progress (turns so far, the tool it is in) is not read from the
        // child yet: the lease is kept alive and the progress left at zero.
        let body = json!({
            "lease_id": lease.lease_id,
            "fencing_token": lease.fencing_token.to_string(),
            "progress_turns": 0, "progress_tool": "",
        });
        let v: Value = self
            .api
            .send_body(
                Method::Post,
                &format!("/v1/coordinator/leases/{}/heartbeat", lease.lease_id),
                &body,
            )
            .await?
            .json()?;
        Ok(LeaseBeat {
            stop: v.get("stop").and_then(Value::as_bool).unwrap_or(false),
            stop_reason: v
                .get("stop_reason")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        })
    }

    async fn report_turn(&self, report: &TurnReport) -> Result<(), ServiceError> {
        let Some(signer) = &self.signer else {
            return Err(ServiceError::NotImplemented(
                "no signing key: run `iohr runner register` on this machine",
            ));
        };
        let signature = signer.sign(super::signing::report_message(report).as_bytes());
        self.api
            .send_body(
                Method::Post,
                &format!("/v1/coordinator/turns/{}/report", report.turn_id),
                &report.to_json(&signature),
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    use super::*;

    #[test]
    fn a_report_goes_on_the_wire_with_the_contracts_shapes() {
        let r = TurnReport {
            turn_id: "t".into(),
            lease_id: "l".into(),
            fencing_token: 9_007_199_254_740_993,
            runner_id: "r".into(),
            pack_sha256: String::new(),
            outcome: "limit_hit".into(),
            say: "hi".into(),
            state: String::new(),
            actions: vec![Action {
                kind: "ask".into(),
                payload_json: "{}".into(),
            }],
            evidence: vec!["e".into()],
            left: vec![],
            provider: "claude".into(),
            provider_session_id: String::new(),
            resumed: false,
            provider_version: String::new(),
            num_turns: 1,
            duration_ms: 2,
            total_cost_usd: None,
            usage: Usage {
                input: 1,
                output: 2,
                cache_read: 3,
                cache_write: 4,
            },
            denied_tools: vec![],
            detail: String::new(),
            attempt: 2,
            resets_at: "2026-10-12T12:00:00Z".into(),
        };
        let j = r.to_json(&[1, 2, 3]);
        // 64-bit integers travel as strings; bytes as standard base64.
        assert_eq!(j["fencing_token"], "9007199254740993");
        assert_eq!(j["reply"]["say"], "hi");
        assert_eq!(j["reply"]["actions"][0]["kind"], "ask");
        assert_eq!(j["reply"]["actions"][0]["payload_json"], "{}");
        assert_eq!(j["reply"]["evidence"][0], "e");
        assert_eq!(j["usage"]["cache_write"], "4");
        assert_eq!(
            (j["attempt"].as_u64(), j["resets_at"].as_str()),
            (Some(2), Some("2026-10-12T12:00:00Z"))
        );
        assert_eq!(j["signature"], "AQID");
        assert_eq!(r.idempotency_key(), "l:9007199254740993:t:2");
    }

    #[test]
    fn a_pack_reads_every_field_of_the_contract() {
        let p = Pack::from_json(&json!({
            "turn_id": "t", "user": "u", "kind": "coordinate", "pack_sha256": "h",
            "system_prompt": "s", "prompt": "p", "model": "m", "max_turns": 5,
            "allowed_tools": ["Read"], "wall_clock_secs": 600,
            "resume_session": "x", "resume_runner_id": "r", "thread_id": "th",
            "job_id": "j", "attempt": 2, "lease_id": "l", "fencing_token": "9",
            "token_budget": 4000, "reply_shape": "{}"
        }));
        assert_eq!(
            (
                p.thread_id.as_str(),
                p.job_id.as_str(),
                p.attempt,
                p.lease_id.as_str()
            ),
            ("th", "j", 2, "l")
        );
        assert_eq!(
            (p.fencing_token, p.token_budget, p.reply_shape.as_str()),
            (9, 4000, "{}")
        );
        assert_eq!((p.max_turns, p.wall_clock_secs), (5, 600));
    }

    #[test]
    fn a_lease_reads_its_fencing_token_as_string_or_number() {
        let a = Lease::from_json(&json!({"lease_id": "l", "fencing_token": "12"}));
        let b = Lease::from_json(&json!({"lease_id": "l", "fencing_token": 12}));
        assert_eq!((a.fencing_token, b.fencing_token), (12, 12));
    }
}
