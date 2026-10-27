//! The coordinator service as the runner sees it, over REST (core PR #879's proto):
//! `CreateRunnerChallenge`, `RegisterRunner`, `ListRunners`, `RunnerHeartbeat`,
//! `AcceptLease`, `GetTurnPack`, `LeaseHeartbeat` and `ReportTurn`.
//!
//! The runner's credential is the CLI profile's own session (`context::Api`): no new
//! secret, no new file.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use inorbithr::Method;
use serde_json::{Value, json};

use super::access::Minted;
use super::settings::Declared;
use super::signing::Signer;
use crate::context::Api;

/// Why a service call did not give an answer.
#[derive(Debug)]
pub(crate) enum ServiceError {
    Api(inorbithr::Error),
    /// Refused here, for good: for example a report with no signing key, or a pack the
    /// service did not send. Never a fake value.
    Refused(&'static str),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Api(e) => write!(f, "{e}"),
            Self::Refused(what) => write!(f, "not implemented: {what}"),
        }
    }
}

impl ServiceError {
    /// A refusal that asking again will not change (the lease is released or reassigned,
    /// the fence is stale, the report breaks a limit): 4xx other than a lapsed sign-in
    /// (401), a timeout (408) and a rate limit (429). Network errors and 5xx are
    /// transient.
    pub(crate) fn is_permanent(&self) -> bool {
        match self {
            Self::Refused(_) => true,
            Self::Api(inorbithr::Error::Api(a)) => {
                (400..500).contains(&a.status) && !matches!(a.status, 401 | 408 | 429)
            }
            Self::Api(_) => false,
        }
    }

    /// A coordinator older than the declared heartbeat: its REST binding refuses a body
    /// field it does not know (`unrecognized field`, HTTP 400).
    pub(crate) fn is_unknown_field(&self) -> bool {
        let Self::Api(inorbithr::Error::Api(a)) = self else {
            return false;
        };
        a.status == 400
            && (a.message.contains("unrecognized field")
                || String::from_utf8_lossy(&a.raw.body).contains("unrecognized field"))
    }
}

impl ServiceError {
    /// The service has no such call: an older coordinator. HTTP 501, or the gateway's own
    /// 404 for a path it has no route for (`no such route`). A 404 that names a missing
    /// job is the service refusing: that stays an error.
    pub(crate) fn is_missing(&self) -> bool {
        let Self::Api(inorbithr::Error::Api(a)) = self else {
            return false;
        };
        a.status == 501
            || (a.status == 404 && a.message.trim().eq_ignore_ascii_case("no such route"))
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
            ServiceError::Refused(_) => Self::Failed(e.to_string()),
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
    /// The Ed25519 public key (base64) the runner registered with; empty if the
    /// service did not say.
    pub(crate) public_key: String,
    /// The turns it was registered to hold at once; 0 when the service did not say.
    pub(crate) capacity: u32,
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
            public_key: s("public_key"),
            capacity: u32::try_from(num(v, "capacity")).unwrap_or(0),
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
    /// Tokens of context (input + cache read + cache creation) above which a session is
    /// never resumed; 0 when the service sent none (no ceiling).
    pub(crate) context_ceiling: u32,
    /// The JSON the final message must be.
    pub(crate) reply_shape: String,
    /// Where the job's branch is checked out, for the Checked gate (JSON `checkout_path`;
    /// `checkoutPath` is read too). The coordinator names it relative to the turn's working
    /// directory (`checkouts/<job_id>`, empty for a job with no repository); the run loop
    /// sends no gate report for a pack without one.
    pub(crate) checkout: String,
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
            context_ceiling: u32::try_from(num(v, "context_ceiling")).unwrap_or(u32::MAX),
            reply_shape: s("reply_shape"),
            // The gateway writes proto field names (`checkout_path`); proto3 JSON's
            // camelCase is read too.
            checkout: Some(s("checkout_path"))
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| s("checkoutPath")),
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

    /// The one journal line a finished turn leaves: its outcome and what it cost, so the
    /// token spend of a runner is readable from `journalctl` without the service. No
    /// reply text and no ids beyond the turn's.
    pub(crate) fn journal_line(&self) -> String {
        let u = &self.usage;
        format!(
            "turn {} ended: {} resumed={} turns={} context={} input={} output={} \
             cache_read={} cache_write={} cost_usd={} ms={}",
            self.turn_id,
            self.outcome,
            self.resumed,
            self.num_turns,
            u.input
                .saturating_add(u.cache_read)
                .saturating_add(u.cache_write),
            u.input,
            u.output,
            u.cache_read,
            u.cache_write,
            self.total_cost_usd
                .map_or_else(|| "none".to_owned(), |c| format!("{c:.4}")),
            self.duration_ms,
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

/// `SubmitGateReportRequest`: the runner's signed report of the check it ran on a build
/// job's PR (RFC 0120, the Checked gate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GateReport {
    pub(crate) job_id: String,
    pub(crate) lease_id: String,
    pub(crate) fencing_token: i64,
    pub(crate) runner_id: String,
    /// The command that ran (1 to 500 characters).
    pub(crate) command: String,
    pub(crate) exit_code: i32,
    /// The PR head it ran on.
    pub(crate) head: String,
    /// Main's commit the head was merged with when it ran.
    pub(crate) main_commit: String,
    /// SHA-256 of the output, lowercase hex. The output itself never leaves the machine.
    pub(crate) output_sha256: String,
}

impl GateReport {
    /// The wire shape of `SubmitGateReport`, carrying `signature` (64 bytes, base64).
    pub(crate) fn to_json(&self, signature: &[u8]) -> Value {
        json!({
            "job_id": self.job_id, "lease_id": self.lease_id,
            "fencing_token": self.fencing_token.to_string(), "runner_id": self.runner_id,
            "command": self.command, "exit_code": self.exit_code,
            "head": self.head, "main_commit": self.main_commit,
            "output_sha256": self.output_sha256,
            "signature": B64.encode(signature),
        })
    }
}

/// What the service answered to a gate report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GateAnswer {
    pub(crate) evidence_id: String,
    /// The same report was recorded before; nothing changed.
    pub(crate) duplicate: bool,
    /// The job's stage after its next gate was asked, when the service said.
    pub(crate) stage: String,
}

impl GateAnswer {
    pub(crate) fn from_json(v: &Value) -> Self {
        let s = |x: &Value, k: &str| {
            x.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        Self {
            evidence_id: s(v, "evidence_id"),
            duplicate: v.get("duplicate").and_then(Value::as_bool).unwrap_or(false),
            stage: v.get("job").map(|j| s(j, "stage")).unwrap_or_default(),
        }
    }
}

/// Keeps at most `n` characters.
fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn plain(s: &str, n: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(n).collect()
}

impl TurnReport {
    /// The report the service will take (core #879, `check_report`): the provider's
    /// words are held to its limits before they are signed. A reply over a limit is
    /// replaced by a bounded `failed` report that says which limit; the shorter fields
    /// are cut to size.
    pub(crate) fn limited(mut self) -> Self {
        let broke = self.reply_limit();
        self.detail = clip(&self.detail, 2000);
        self.provider_session_id = plain(&self.provider_session_id, 200);
        self.provider_version = plain(&self.provider_version, 200);
        self.denied_tools = self
            .denied_tools
            .iter()
            .take(20)
            .map(|t| plain(t, 200))
            .collect();
        if self
            .total_cost_usd
            .is_some_and(|c| !c.is_finite() || c < 0.0)
        {
            self.total_cost_usd = None;
        }
        if let Some(why) = broke {
            self.outcome = "failed".into();
            self.say = String::new();
            self.state = String::new();
            self.actions = vec![];
            self.evidence = vec![];
            self.left = vec![];
            self.detail = clip(
                &format!("the reply broke the service's limits: {why}"),
                2000,
            );
        }
        self
    }

    /// Which limit of the reply the report breaks, if any.
    fn reply_limit(&self) -> Option<&'static str> {
        if self.say.chars().count() > 20_000 || self.state.chars().count() > 16_000 {
            return Some("say over 20,000 characters or state over 16,000");
        }
        if self.actions.len() > 20 {
            return Some("more than 20 actions");
        }
        if self.actions.iter().any(|a| {
            a.kind.is_empty()
                || a.kind.len() > 40
                || !a.kind.chars().all(|c| c.is_ascii_lowercase() || c == '_')
        }) {
            return Some("an action kind is not lower-case letters and underscores, 40 at most");
        }
        if self.actions.iter().any(|a| {
            a.payload_json.len() > 16_384
                || (!a.payload_json.is_empty()
                    && serde_json::from_str::<Value>(&a.payload_json).is_err())
        }) {
            return Some("an action payload is over 16 KiB or not JSON");
        }
        if self.evidence.len() > 50
            || self.left.len() > 50
            || self
                .evidence
                .iter()
                .chain(&self.left)
                .any(|e| e.chars().count() > 2000)
        {
            return Some("more than 50 evidence or left lines, or one over 2,000 characters");
        }
        None
    }
}

/// What the runner reads of a job before it accepts its lease: the repository its
/// checkout is made from (empty for a job without one) and the document it serves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct JobInfo {
    pub(crate) repository: String,
    pub(crate) document_id: String,
}

impl JobInfo {
    pub(crate) fn from_json(v: &Value) -> Self {
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_owned()
        };
        Self {
            repository: s("repository"),
            document_id: s("document_id"),
        }
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
    /// The runner is alive, has these providers, and declares its capacity, version and
    /// kinds (a coordinator older than that is sent the providers alone).
    async fn heartbeat(
        &self,
        runner_id: &str,
        providers: &[ProviderState],
        declared: &Declared,
    ) -> Result<Beat, ServiceError>;
    async fn accept(&self, lease: &Lease, facts: AcceptFacts) -> Result<(), ServiceError>;
    async fn fetch_pack(&self, lease: &Lease) -> Result<Pack, ServiceError>;
    /// The job a lease is for (`GetJob`): its repository, read before the lease is
    /// accepted so the checkout exists when the turn starts. The pack does not carry the
    /// repository yet (RFC 0147 slice B adds `TurnPack.workspace`).
    async fn get_job(&self, job_id: &str) -> Result<JobInfo, ServiceError>;
    /// The lease is alive; says whether the turn must stop.
    async fn lease_heartbeat(&self, lease: &Lease) -> Result<LeaseBeat, ServiceError>;
    /// Delivers a finished turn, signed; the same report again is answered as the first
    /// was (idempotent on the turn and its attempt).
    async fn report_turn(&self, report: &TurnReport) -> Result<(), ServiceError>;
    /// Delivers the signed report of a build job's check. A service older than the
    /// gate report answers 404 or 501; that is [`ServiceError::is_missing`], and the
    /// caller says so and goes on (the job holds at Checked, as it did).
    async fn submit_gate_report(&self, report: &GateReport) -> Result<GateAnswer, ServiceError>;
    /// A GitHub App installation token for the one repository of this lease, minted by
    /// connections (`MintRunnerGithubToken`, RFC 0120) after the coordinator confirmed the
    /// lease is this person's and live at its fencing token. A platform older than the
    /// call answers [`ServiceError::is_missing`]. The answer's token never prints
    /// ([`Minted`]'s `Debug`); the response body is dropped once read.
    async fn runner_github_token(&self, lease: &Lease) -> Result<Minted, ServiceError>;
}

/// The service over REST, called as the profile's own session.
///
/// Every call names `account_id`. The sign-in token's `org` claim is always the person's
/// personal account, and the service takes the account of a request from `account_id`
/// (a team the person is in) before it falls back to the token's: a call without it acts
/// in the personal account whatever account the profile names.
#[derive(Debug)]
pub(crate) struct HttpService<'a> {
    pub(crate) api: &'a Api,
    /// The account the runner belongs to: `--account`, else the profile's.
    pub(crate) account: String,
    /// The runner's key, to sign reports with; absent for commands that report nothing.
    pub(crate) signer: Option<Signer>,
    /// Set once the coordinator refused the declared heartbeat as an older one: every
    /// later beat sends the providers alone.
    old_beat: AtomicBool,
}

impl<'a> HttpService<'a> {
    pub(crate) fn new(api: &'a Api, account: String, signer: Option<Signer>) -> Self {
        Self {
            api,
            account,
            signer,
            old_beat: AtomicBool::new(false),
        }
    }
    /// A request body with this service's `account_id` in it.
    fn scoped(&self, mut body: Value) -> Value {
        if let Value::Object(m) = &mut body {
            m.insert("account_id".into(), Value::String(self.account.clone()));
        }
        body
    }
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
                &self.scoped(json!({})),
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
            .send_body(Method::Post, "/v1/coordinator/runners", &self.scoped(body))
            .await?
            .json()?;
        Ok(RunnerInfo::from_json(
            v.get("runner").unwrap_or(&Value::Null),
        ))
    }

    async fn list_runners(&self, mine: bool) -> Result<Vec<RunnerInfo>, ServiceError> {
        let mut q: Vec<(&str, &str)> = vec![("account_id", self.account.as_str())];
        if mine {
            q.push(("mine", "true"));
        }
        let v: Value = self.api.get("/v1/coordinator/runners", &q).await?;
        Ok(v.get("runners")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(RunnerInfo::from_json).collect())
            .unwrap_or_default())
    }

    async fn heartbeat(
        &self,
        runner_id: &str,
        providers: &[ProviderState],
        declared: &Declared,
    ) -> Result<Beat, ServiceError> {
        let path = format!("/v1/coordinator/runners/{runner_id}/heartbeat");
        let old = || {
            json!({
                "runner_id": runner_id,
                "providers": providers.iter().map(ProviderState::to_json).collect::<Vec<_>>(),
            })
        };
        let answer = if self.old_beat.load(Ordering::Relaxed) {
            self.api
                .send_body(Method::Post, &path, &self.scoped(old()))
                .await
                .map_err(ServiceError::from)
        } else {
            let mut body = old();
            if let Value::Object(m) = &mut body {
                if let Some(c) = declared.capacity {
                    m.insert("capacity".into(), json!(c));
                }
                m.insert("version".into(), json!(declared.version));
                m.insert("kinds".into(), json!(declared.kinds));
                // 64-bit integers travel as strings.
                if let Some(b) = declared.disk_free_bytes {
                    m.insert("disk_free_bytes".into(), json!(b.to_string()));
                }
                if let Some(r) = &declared.lease_paused_reason {
                    m.insert("lease_paused_reason".into(), json!(r));
                }
            }
            match self
                .api
                .send_body(Method::Post, &path, &self.scoped(body))
                .await
            {
                Ok(r) => Ok(r),
                Err(e) => match ServiceError::from(e) {
                    e if e.is_unknown_field() => {
                        // An older coordinator: it keeps the registered capacity and kinds.
                        self.old_beat.store(true, Ordering::Relaxed);
                        self.api
                            .send_body(Method::Post, &path, &self.scoped(old()))
                            .await
                            .map_err(ServiceError::from)
                    }
                    e => Err(e),
                },
            }
        };
        let v: Value = answer?.json()?;
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
                &self.scoped(body),
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
                &[
                    ("fencing_token", token.as_str()),
                    ("account_id", self.account.as_str()),
                ],
            )
            .await?;
        let pack = v.get("pack").unwrap_or(&Value::Null);
        if pack.is_null() {
            return Err(ServiceError::Refused("the service sent no pack"));
        }
        Ok(Pack::from_json(pack))
    }

    async fn get_job(&self, job_id: &str) -> Result<JobInfo, ServiceError> {
        if job_id.is_empty()
            || !job_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(ServiceError::Refused("the lease names no usable job id"));
        }
        let v: Value = self
            .api
            .get(
                &format!("/v1/coordinator/jobs/{job_id}"),
                &[("account_id", self.account.as_str())],
            )
            .await?;
        let job = v.get("job").unwrap_or(&Value::Null);
        if job.is_null() {
            return Err(ServiceError::Refused("the service sent no job"));
        }
        Ok(JobInfo::from_json(job))
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
                &self.scoped(body),
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
            return Err(ServiceError::Refused(
                "no signing key: run `iohr runner register` on this machine",
            ));
        };
        let signature = signer.sign(super::signing::report_message(report).as_bytes());
        self.api
            .send_body(
                Method::Post,
                &format!("/v1/coordinator/turns/{}/report", report.turn_id),
                &self.scoped(report.to_json(&signature)),
            )
            .await?;
        Ok(())
    }

    async fn submit_gate_report(&self, report: &GateReport) -> Result<GateAnswer, ServiceError> {
        let Some(signer) = &self.signer else {
            return Err(ServiceError::Refused(
                "no signing key: run `iohr runner register` on this machine",
            ));
        };
        let signature = signer.sign(super::signing::gate_report_message(report).as_bytes());
        let v: Value = self
            .api
            .send_body(
                Method::Post,
                &format!("/v1/coordinator/jobs/{}/gate-report", report.job_id),
                &self.scoped(report.to_json(&signature)),
            )
            .await?
            .json()?;
        Ok(GateAnswer::from_json(&v))
    }

    async fn runner_github_token(&self, lease: &Lease) -> Result<Minted, ServiceError> {
        if self.account.is_empty()
            || !self
                .account
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(ServiceError::Refused("the runner names no usable account"));
        }
        let body = json!({
            "lease_id": lease.lease_id,
            "fencing_token": lease.fencing_token.to_string(),
        });
        let v: Value = self
            .api
            .send_body(
                Method::Post,
                &format!(
                    "/v1/accounts/orgs/{}/connections/github/runner-token",
                    self.account
                ),
                &body,
            )
            .await?
            .json()?;
        Ok(Minted::from_json(&v))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    use super::*;
    type Break = Box<dyn Fn(&mut super::TurnReport)>;

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
    fn a_finished_turn_leaves_its_usage_in_one_journal_line() {
        let mut r = valid_report();
        r.usage = Usage {
            input: 10,
            output: 89,
            cache_read: 1000,
            cache_write: 17_676,
        };
        r.num_turns = 2;
        r.duration_ms = 2053;
        r.total_cost_usd = Some(0.035_807);
        assert_eq!(
            r.journal_line(),
            "turn t ended: replied resumed=false turns=2 context=18686 input=10 output=89 \
             cache_read=1000 cache_write=17676 cost_usd=0.0358 ms=2053"
        );
        r.total_cost_usd = None;
        assert!(r.journal_line().contains("cost_usd=none"));
    }

    fn valid_report() -> TurnReport {
        TurnReport {
            turn_id: "t".into(),
            lease_id: "l".into(),
            fencing_token: 1,
            runner_id: "r".into(),
            pack_sha256: "a".repeat(64),
            outcome: "replied".into(),
            say: "hi".into(),
            state: String::new(),
            actions: vec![Action {
                kind: "create_job".into(),
                payload_json: "{}".into(),
            }],
            evidence: vec![],
            left: vec![],
            provider: "claude".into(),
            provider_session_id: "s".into(),
            resumed: false,
            provider_version: "1".into(),
            num_turns: 1,
            duration_ms: 1,
            total_cost_usd: Some(0.1),
            usage: Usage::default(),
            denied_tools: vec![],
            detail: String::new(),
            attempt: 1,
            resets_at: String::new(),
        }
    }

    #[test]
    fn a_reply_over_the_services_limits_becomes_a_bounded_failure() {
        let ok = valid_report().limited();
        assert_eq!((ok.outcome.as_str(), ok.say.as_str()), ("replied", "hi"));
        let bad: Vec<(&str, Break)> = vec![
            ("say", Box::new(|r| r.say = "x".repeat(20_001))),
            ("state", Box::new(|r| r.state = "x".repeat(16_001))),
            (
                "21 actions",
                Box::new(|r| r.actions = vec![r.actions[0].clone(); 21]),
            ),
            (
                "kind caps",
                Box::new(|r| r.actions[0].kind = "Create".into()),
            ),
            (
                "kind empty",
                Box::new(|r| r.actions[0].kind = String::new()),
            ),
            (
                "kind long",
                Box::new(|r| r.actions[0].kind = "a".repeat(41)),
            ),
            (
                "payload big",
                Box::new(|r| r.actions[0].payload_json = format!("\"{}\"", "x".repeat(16_400))),
            ),
            (
                "payload not json",
                Box::new(|r| r.actions[0].payload_json = "{".into()),
            ),
            (
                "51 evidence",
                Box::new(|r| r.evidence = vec!["e".into(); 51]),
            ),
            ("long left", Box::new(|r| r.left = vec!["e".repeat(2001)])),
        ];
        for (name, f) in bad {
            let mut r = valid_report();
            f(&mut r);
            let r = r.limited();
            assert_eq!(r.outcome, "failed", "{name}");
            assert!(r.say.is_empty() && r.actions.is_empty(), "{name}");
            assert!(
                r.detail.contains("limits") && r.detail.chars().count() <= 2000,
                "{name}"
            );
            // Still the pack's hash and attempt: only the words are replaced.
            assert_eq!((r.pack_sha256.len(), r.attempt), (64, 1));
        }
    }

    #[test]
    fn the_short_fields_are_cut_to_size_not_failed() {
        let mut r = valid_report();
        r.detail = "d".repeat(5000);
        r.provider_version = format!("1\n{}", "v".repeat(300));
        r.denied_tools = vec!["Bash".into(); 30];
        r.total_cost_usd = Some(f64::NAN);
        let r = r.limited();
        assert_eq!(r.outcome, "replied");
        assert_eq!(r.detail.chars().count(), 2000);
        assert_eq!(r.provider_version.chars().count(), 200);
        assert!(!r.provider_version.contains('\n'));
        assert_eq!((r.denied_tools.len(), r.total_cost_usd), (20, None));
    }

    #[test]
    fn a_4xx_is_permanent_a_lapsed_sign_in_a_timeout_and_the_network_are_not() {
        assert!(ServiceError::Refused("x").is_permanent());
        let timeout = ServiceError::Api(inorbithr::Error::Timeout {
            host: "h".into(),
            secs: 1,
        });
        assert!(!timeout.is_permanent());
    }

    fn api_error(status: u16, body: &str) -> ServiceError {
        let raw = inorbithr::RawResponse::for_tests(
            status,
            inorbithr::Headers::new([]),
            body.as_bytes().to_vec(),
        );
        ServiceError::Api(inorbithr::Error::Api(Box::new(inorbithr::ApiError::parse(
            raw,
        ))))
    }

    #[test]
    fn only_a_missing_route_or_a_501_means_an_older_coordinator() {
        let gateway = r#"{"code":"not_found","error":"no such route","details":[]}"#;
        assert!(api_error(404, gateway).is_missing());
        assert!(api_error(501, "").is_missing());
        // The service saying the job is not there is a refusal, not an old coordinator.
        let job = r#"{"code":"not_found","error":"no such job","details":[]}"#;
        assert!(!api_error(404, job).is_missing());
        assert!(!api_error(403, gateway).is_missing());
        assert!(!api_error(500, "").is_missing());
        assert!(!ServiceError::Refused("x").is_missing());
    }

    #[test]
    fn a_gate_report_goes_out_with_a_string_token_a_number_exit_and_a_base64_signature() {
        let r = GateReport {
            job_id: "job_1".into(),
            lease_id: "lse_1".into(),
            fencing_token: 9_007_199_254_740_993,
            runner_id: "rnr_1".into(),
            command: "mise run ci:changed".into(),
            exit_code: 2,
            head: "a".repeat(40),
            main_commit: "b".repeat(40),
            output_sha256: "c".repeat(64),
        };
        let v = r.to_json(&[1_u8; 64]);
        assert_eq!(v["fencing_token"], "9007199254740993");
        assert_eq!(v["exit_code"], 2);
        assert_eq!(v["signature"].as_str().map(str::len), Some(88));
        assert_eq!(v["output_sha256"], "c".repeat(64));
        // Nothing but the report's own fields: the output never rides along.
        assert_eq!(v.as_object().map(serde_json::Map::len), Some(10));
        let a = GateAnswer::from_json(&json!({
            "evidence_id": "evd_1", "duplicate": true, "job": {"stage": "checked"}
        }));
        assert_eq!((a.evidence_id.as_str(), a.duplicate), ("evd_1", true));
    }

    #[test]
    fn a_pack_reads_every_field_of_the_contract() {
        let p = Pack::from_json(&json!({
            "turn_id": "t", "user": "u", "kind": "coordinate", "pack_sha256": "h",
            "system_prompt": "s", "prompt": "p", "model": "m", "max_turns": 5,
            "allowed_tools": ["Read"], "wall_clock_secs": 600,
            "resume_session": "x", "resume_runner_id": "r", "thread_id": "th",
            "job_id": "j", "attempt": 2, "lease_id": "l", "fencing_token": "9",
            "token_budget": 4000, "context_ceiling": 120_000, "reply_shape": "{}",
            "checkout_path": "checkouts/job_1"
        }));
        assert_eq!(p.checkout, "checkouts/job_1");
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
    fn a_pack_reads_its_checkout_under_either_json_name() {
        // What the gateway sends today (proto field names) and proto3 JSON's camelCase.
        let snake = Pack::from_json(&json!({"checkout_path": "checkouts/j"}));
        let camel = Pack::from_json(&json!({"checkoutPath": "checkouts/j"}));
        let none = Pack::from_json(&json!({"checkout_path": ""}));
        assert_eq!(
            (
                snake.checkout.as_str(),
                camel.checkout.as_str(),
                none.checkout.as_str()
            ),
            ("checkouts/j", "checkouts/j", "")
        );
    }

    #[test]
    fn a_job_reads_its_repository_and_document() {
        let j = JobInfo::from_json(&serde_json::json!({
            "id": "job_1", "repository": " inorbithr/core ", "document_id": "ldoc_1", "kind": "api"
        }));
        assert_eq!(j.repository, "inorbithr/core");
        assert_eq!(j.document_id, "ldoc_1");
        assert_eq!(
            JobInfo::from_json(&serde_json::json!({})),
            JobInfo::default()
        );
    }

    #[tokio::test]
    async fn the_runner_token_is_asked_for_by_lease_and_never_shows_in_debug_or_errors() {
        use iohr_auth::{Redacted, StaticToken};
        use std::sync::Arc;
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::super::access::tests::FAKE;
        use crate::context::AnyCredential;

        let server = MockServer::start().await;
        let route = "/v1/accounts/orgs/acc_1/connections/github/runner-token";
        Mock::given(method("POST"))
            .and(path(route))
            .and(body_json(
                json!({"lease_id": "lse_1", "fencing_token": "9007199254740993"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "token": FAKE, "expires_at": "2099-01-01T00:00:00Z",
                "repository": "acme/app", "reason": "", "detail": ""
            })))
            .expect(1)
            .mount(&server)
            .await;
        let person = Arc::new(AnyCredential::Token(StaticToken::new(Redacted::new(
            "person-token".to_owned(),
        ))));
        let api = Api::new(&server.uri(), person, false).unwrap();
        let svc = HttpService::new(&api, "acc_1".into(), None);
        let lease =
            Lease::from_json(&json!({"lease_id": "lse_1", "fencing_token": "9007199254740993"}));
        let m = svc.runner_github_token(&lease).await.unwrap();
        assert!(!format!("{m:?}").contains(FAKE), "the token in Debug");
        let a = m
            .access("acme/app", time::OffsetDateTime::now_utc())
            .unwrap();
        assert_eq!(a.gh_token(), Some(FAKE));
        assert!(!format!("{a:?}").contains(FAKE) && !a.describe().contains(FAKE));

        // A refusal says the platform's words; an older platform is `is_missing`.
        let old = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(404).set_body_raw(
                r#"{"code":"not_found","error":"no such route","details":[]}"#,
                "application/json",
            ))
            .mount(&old)
            .await;
        let person = Arc::new(AnyCredential::Token(StaticToken::new(Redacted::new(
            "person-token".to_owned(),
        ))));
        let api = Api::new(&old.uri(), person, false).unwrap();
        let svc = HttpService::new(&api, "acc_1".into(), None);
        let e = svc.runner_github_token(&lease).await.unwrap_err();
        assert!(e.is_missing(), "{e}");
        assert!(!e.to_string().contains(FAKE) && !format!("{e:?}").contains(FAKE));
        let bad = HttpService::new(&api, "../x".into(), None);
        assert!(
            bad.runner_github_token(&lease)
                .await
                .unwrap_err()
                .is_permanent()
        );
    }

    #[test]
    fn a_lease_reads_its_fencing_token_as_string_or_number() {
        let a = Lease::from_json(&json!({"lease_id": "l", "fencing_token": "12"}));
        let b = Lease::from_json(&json!({"lease_id": "l", "fencing_token": 12}));
        assert_eq!((a.fencing_token, b.fencing_token), (12, 12));
    }
}
