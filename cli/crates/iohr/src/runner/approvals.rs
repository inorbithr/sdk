//! Approved pinned runs (platform RFC 0153, RFC 0172 slice 3): the runner carries out
//! the host actions a person approved for it, and nothing else.
//!
//! Beside the coordinator loop, `iohr runner run` polls the approvals service
//! (`GET /v1/approvals?status=approved&kind=run`) for runs pinned to this runner. Before
//! it acts on one it checks, in core's order (`approval_messages::pinned::verify`, ported
//! here because this crate cannot link core): the run is pinned; its payload hashes to the
//! recorded `payload_sha256`; a person decided it; its host action validates against
//! `runner/host-action/v0` and is the one pinned; it pins this runner's id and key; it
//! has not expired; and the decision's proof verifies against a device key this machine
//! pins locally (`<config>/runner/approvals/<account>/device-keys`), never on the
//! service's word. Then the proof's nonce must be new to this machine
//! (`<config>/runner/approvals/<account>/seen`). Both files must belong to the user and
//! not be group- or world-writable ([`super::private`]).
//!
//! A run that fails a check is reported `failed` with the reason and nothing runs.
//! Otherwise the runner records the nonce (on disk, under a file lock, before anything
//! else), reports `started` (with the payload as it saw it), carries out the action
//! ([`host_action`]) and reports `done` or `failed` with the output, redacted and cut to
//! the service's 500-character note. Every report is signed with the runner's P-256 run
//! key over `iohr.approvals.v1.ReportRun` ([`signing::run_report_message`]) at the time
//! it is made.
//!
//! Duplicates to fold later: this is the second runner registry (the coordinator's
//! Ed25519 runners and the approvals service's P-256 runners), and `canonical`,
//! `DeviceKeys` and `verify` are ports of core's `approval_messages`, which the roll runner
//! also carries its own copy of.

use std::collections::{BTreeMap, HashSet};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use aws_lc_rs::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use inorbithr::Method;
use serde_json::{Value, json};
use tokio::sync::watch;

use super::host_action::{self, ArgvRunner, HOST_ACTION};
use super::private;
use super::service::{HttpService, ServiceError};
use super::signing::{RunKey, h as sha256_hex, run_report_message};

/// The first line of a decision's proof (core `approval_messages::DECISION_TAG`).
pub(crate) const DECISION_TAG: &str = "iohr-approval-decision-v1";

/// The longest note the service keeps, in characters.
pub(crate) const NOTE_MAX: usize = 500;

/// How often the approvals service is asked for new approved runs.
pub(crate) const POLL: Duration = Duration::from_secs(20);

/// The longest wait between polls after failures.
const POLL_MAX: Duration = Duration::from_secs(300);

/// The most pages of approved runs read in one poll.
const MAX_PAGES: usize = 10;

/// JSON with object keys sorted at every level and no white space (core
/// `approval_messages::canonical`).
pub(crate) fn canonical(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let sorted: BTreeMap<&String, &Value> = m.iter().collect();
            let inner: Vec<String> = sorted
                .into_iter()
                .map(|(k, v)| format!("{}:{}", Value::String(k.clone()), canonical(v)))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        other => other.to_string(),
    }
}

/// SHA-256 hex of the payload's canonical JSON.
pub(crate) fn payload_digest(payload: &Value) -> String {
    sha256_hex(&canonical(payload))
}

fn payload_hashes_to(payload: &Value, claimed: &str) -> bool {
    !claimed.is_empty() && payload_digest(payload) == claimed
}

/// Whether `decided_by` names a person: our own services (`svc:`) never are one.
fn decided_by_person(decided_by: &str) -> bool {
    let who = decided_by.trim();
    !who.is_empty() && !who.starts_with("svc:")
}

/// What a device key signs to approve (core `approval_messages::decision`).
pub(crate) fn decision_message(
    approval_id: &str,
    payload_sha256: &str,
    decision: &str,
    note: &str,
    signed_at: i64,
    nonce: &str,
    key_id: &str,
) -> String {
    [
        DECISION_TAG,
        approval_id,
        payload_sha256,
        decision,
        &sha256_hex(note.trim()),
        &signed_at.to_string(),
        nonce.trim(),
        key_id.trim(),
    ]
    .join("\n")
}

fn unb64(text: &str) -> Option<Vec<u8>> {
    B64URL.decode(text.trim().trim_end_matches('=')).ok()
}

/// The owner's device keys this runner trusts, by key id. The service cannot list them
/// (a person's keys carry no public key over the API), so the runner pins them in a
/// file: one `key_id public_key` per line, the key base64url of the 65-byte P-256 point;
/// `#` starts a comment.
#[derive(Debug, Clone, Default)]
pub(crate) struct DeviceKeys(BTreeMap<String, Vec<u8>>);

impl DeviceKeys {
    /// Trust `public` as `key_id`.
    ///
    /// # Errors
    /// The key is not base64url of an uncompressed P-256 point.
    pub(crate) fn insert(&mut self, key_id: &str, public: &str) -> Result<(), String> {
        let point = unb64(public).ok_or_else(|| format!("device key {key_id}: not base64url"))?;
        if point.len() != 65 || point[0] != 0x04 {
            return Err(format!(
                "device key {key_id}: not an uncompressed P-256 point"
            ));
        }
        self.0.insert(key_id.trim().to_owned(), point);
        Ok(())
    }

    /// The keys in `text`.
    ///
    /// # Errors
    /// A line is not two fields, or a key is not a P-256 point.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let mut keys = Self::default();
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut fields = line.split_whitespace();
            let (Some(id), Some(public), None) = (fields.next(), fields.next(), fields.next())
            else {
                return Err("device keys: each line is `key_id public_key`".to_owned());
            };
            keys.insert(id, public)?;
        }
        Ok(keys)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether `signature` (base64url DER) over `message` is the key `key_id`'s.
    pub(crate) fn verifies(&self, key_id: &str, message: &str, signature: &str) -> bool {
        let (Some(point), Some(sig)) = (self.0.get(key_id.trim()), unb64(signature)) else {
            return false;
        };
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, point)
            .verify(message.as_bytes(), &sig)
            .is_ok()
    }
}

/// A 64-bit integer as REST sends it (a string) or as JSON holds it (a number).
fn int(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::String(t)) => t.trim().parse().unwrap_or(0),
        Some(n) => n.as_i64().unwrap_or(0),
        None => 0,
    }
}

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// A decision's device-key proof.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Proof {
    pub(crate) key_id: String,
    pub(crate) signed_at: i64,
    pub(crate) nonce: String,
    pub(crate) signature: String,
}

/// An approval as the record reads (the payload goes beside it).
#[derive(Debug, Clone, Default)]
pub(crate) struct Approved {
    pub(crate) approval_id: String,
    pub(crate) kind: String,
    pub(crate) status: String,
    pub(crate) decided_by: String,
    pub(crate) payload_sha256: String,
    pub(crate) decision_note: String,
    pub(crate) proof: Option<Proof>,
}

impl Approved {
    /// An `Approval` as REST returns it (or a test vector holds it).
    pub(crate) fn from_json(v: &Value) -> Self {
        let proof = v
            .get("decision_proof")
            .filter(|p| p.is_object())
            .map(|p| Proof {
                key_id: text(p, "key_id"),
                signed_at: int(p.get("signed_at")),
                nonce: text(p, "nonce"),
                signature: text(p, "signature"),
            });
        Self {
            approval_id: text(v, "approval_id"),
            kind: text(v, "kind"),
            status: text(v, "status"),
            decided_by: text(v, "decided_by"),
            payload_sha256: text(v, "payload_sha256"),
            decision_note: text(v, "decision_note"),
            proof,
        }
    }
}

/// The runner about to act: its enrolled approvals id and its run key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pin {
    pub(crate) runner_id: String,
    pub(crate) runner_public_key: String,
}

/// Why an approval must not be acted on; the labels are core's and the vectors'.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    NotApproved(String),
    NotPinned(String),
    HashMismatch,
    NoApprover,
    SchemaInvalid(String),
    WrongRunner,
    Expired,
    BadSignature(String),
    /// This decision's nonce, or this approval, was acted on before (this runner only).
    Replay,
    /// A pinned run this runner does not carry out (a script or a roll train).
    NotHostAction(String),
}

impl Refusal {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::NotApproved(_) => "not_approved",
            Self::NotPinned(_) => "not_pinned",
            Self::HashMismatch => "hash_mismatch",
            Self::NoApprover => "no_approver",
            Self::SchemaInvalid(_) => "schema_invalid",
            Self::WrongRunner => "wrong_runner",
            Self::Expired => "expired",
            Self::BadSignature(_) => "bad_signature",
            Self::Replay => "replay",
            Self::NotHostAction(_) => "not_host_action",
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotApproved(w) => write!(f, "the approval is not an approved run ask: {w}"),
            Self::NotPinned(w) => write!(f, "the approval is not a pinned run: {w}"),
            Self::HashMismatch => {
                f.write_str("the approval's payload does not match its recorded hash")
            }
            Self::NoApprover => f.write_str("the approval has no person's decision"),
            Self::SchemaInvalid(w) => write!(f, "the approval's host action is refused: {w}"),
            Self::WrongRunner => f.write_str("the approval pins another runner"),
            Self::Expired => f.write_str("the approval has expired"),
            Self::BadSignature(w) => write!(f, "the decision's proof does not verify: {w}"),
            Self::Replay => f.write_str("this decision was acted on before; it is not run again"),
            Self::NotHostAction(w) => write!(f, "this runner carries out host actions only: {w}"),
        }
    }
}

fn has_float(v: &Value) -> bool {
    match v {
        Value::Number(n) => !(n.is_i64() || n.is_u64()),
        Value::Array(a) => a.iter().any(has_float),
        Value::Object(m) => m.values().any(has_float),
        _ => false,
    }
}

/// The classes of a run's own `effect` (`approvals.proto`, `Run.effect`). Never the action's
/// sentence: that one is the action's own `effect`, held to the catalogue by `validate`.
const EFFECT_CLASSES: [&str; 4] = ["read_only", "local_write", "remote", "production"];

/// Whether `action` is a host action this run may carry (core `check_action`), and `effect`,
/// the run's own, is one of the [`EFFECT_CLASSES`].
///
/// # Errors
/// What is wrong, naming fields only.
pub(crate) fn check_action(
    schema_id: &str,
    action: &Value,
    script_sha256: &str,
    argv: &[String],
    runner_id: &str,
    effect: &str,
) -> Result<(), String> {
    if schema_id != HOST_ACTION {
        return Err(format!("schema_id: only {HOST_ACTION}"));
    }
    if has_float(action) {
        return Err("action: integers and strings only".to_owned());
    }
    host_action::validate(action)
        .map_err(|at| format!("action: not a {HOST_ACTION} document (at {at})"))?;
    if payload_digest(action) != script_sha256 {
        return Err("script_sha256: not the SHA-256 of the action's canonical JSON".to_owned());
    }
    if !EFFECT_CLASSES.contains(&effect) {
        return Err("effect: not read_only, local_write, remote or production".to_owned());
    }
    let id = action["action"].as_str().unwrap_or_default();
    if argv.len() != 1 || argv[0] != id {
        return Err("argv: exactly the action's id".to_owned());
    }
    if action["runner_id"].as_str() != Some(runner_id) {
        return Err("action.runner_id: not the run's runner".to_owned());
    }
    Ok(())
}

/// Whether `approval`, with its `payload` as hashed, may be acted on by `pin` at `now`
/// (unix seconds), its proof checked against `keys`: core's `verify` with a runner pin
/// and device keys, in core's order.
///
/// # Errors
/// The first [`Refusal`] that applies.
pub(crate) fn verify(
    approval: &Approved,
    payload: &Value,
    pin: &Pin,
    keys: &DeviceKeys,
    now: i64,
) -> Result<(), Refusal> {
    if approval.kind != "run" {
        return Err(Refusal::NotApproved(format!(
            "kind \"{}\", not run",
            approval.kind
        )));
    }
    if approval.status != "approved" {
        return Err(Refusal::NotApproved(format!(
            "status \"{}\", not approved",
            approval.status
        )));
    }
    let (script, runner) = (text(payload, "script_sha256"), text(payload, "runner_id"));
    if script.is_empty() || runner.is_empty() || text(payload, "runner_public_key").is_empty() {
        return Err(Refusal::NotPinned(
            "it names no script snapshot, runner or runner key".to_owned(),
        ));
    }
    let expires_at = payload["expires_at"].as_i64().unwrap_or(0);
    if expires_at <= 0 {
        return Err(Refusal::NotPinned("it has no signed expiry".to_owned()));
    }
    if !payload_hashes_to(payload, &approval.payload_sha256) {
        return Err(Refusal::HashMismatch);
    }
    if !decided_by_person(&approval.decided_by) {
        return Err(Refusal::NoApprover);
    }
    match (payload.get("schema_id"), payload.get("action")) {
        (None, None) => {}
        (Some(schema), Some(action)) => {
            let argv: Vec<String> = payload["argv"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|s| s.as_str().unwrap_or_default().to_owned())
                        .collect()
                })
                .unwrap_or_default();
            check_action(
                schema.as_str().unwrap_or_default(),
                action,
                &script,
                &argv,
                &runner,
                &text(payload, "effect"),
            )
            .map_err(Refusal::SchemaInvalid)?;
        }
        _ => {
            return Err(Refusal::SchemaInvalid(
                "schema_id and action go together".to_owned(),
            ));
        }
    }
    if runner != pin.runner_id || text(payload, "runner_public_key") != pin.runner_public_key {
        return Err(Refusal::WrongRunner);
    }
    if expires_at <= now {
        return Err(Refusal::Expired);
    }
    let Some(proof) = approval.proof.as_ref().filter(|p| !p.signature.is_empty()) else {
        return Err(Refusal::BadSignature(
            "the decision carries no proof".to_owned(),
        ));
    };
    if proof.nonce.trim().is_empty() {
        return Err(Refusal::BadSignature("the proof has no nonce".to_owned()));
    }
    let message = decision_message(
        &approval.approval_id,
        &approval.payload_sha256,
        "approve",
        &approval.decision_note,
        proof.signed_at,
        &proof.nonce,
        &proof.key_id,
    );
    if !keys.verifies(&proof.key_id, &message, &proof.signature) {
        return Err(Refusal::BadSignature(
            "not a pinned device key's signature".to_owned(),
        ));
    }
    Ok(())
}

/// A run payload as REST sends it (`payload.run`: 64-bit integers as strings, every field
/// present, the action as JSON text) in the form it was hashed (core
/// `PinnedRun::to_json`): `cmd` from `argv`, `plan_sha256` only when set, `version` and
/// `nonce` only from version 2, `schema_id` and the action (an object) only when set.
///
/// # Errors
/// A roll train (this runner does not carry them out), or an action that is not JSON.
pub(crate) fn stored_payload(run: &Value) -> Result<Value, Refusal> {
    let argv: Vec<Value> = run
        .get("argv")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let words: Vec<&str> = argv
        .iter()
        .map(|a| a.as_str().unwrap_or_default())
        .collect();
    let mut v = json!({
        "cmd": words.join(" "),
        "argv": argv,
        "script_sha256": text(run, "script_sha256"),
        "effect": text(run, "effect"),
        "runner_id": text(run, "runner_id"),
        "runner_public_key": text(run, "runner_public_key"),
        "expires_at": int(run.get("expires_at")),
    });
    let plan = text(run, "plan_sha256");
    if !plan.is_empty() {
        v["plan_sha256"] = json!(plan);
    }
    let version = int(run.get("version"));
    if version >= 2 {
        v["version"] = json!(version);
        v["nonce"] = json!(text(run, "nonce"));
    }
    if run.get("roll").is_some_and(|r| !r.is_null()) {
        return Err(Refusal::NotHostAction("a roll train".to_owned()));
    }
    let schema_id = text(run, "schema_id");
    if !schema_id.is_empty() {
        v["schema_id"] = json!(schema_id);
    }
    let action = text(run, "action");
    if !action.is_empty() {
        let doc: Value = serde_json::from_str(&action)
            .map_err(|_| Refusal::SchemaInvalid("action: not JSON".to_owned()))?;
        v["action"] = doc;
    }
    Ok(v)
}

/// The note the service will keep: no control character but new lines and tabs, at
/// most [`NOTE_MAX`] characters, trimmed (labs `prose`).
pub(crate) fn note_text(s: &str) -> String {
    let kept: String = s
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .take(NOTE_MAX)
        .collect();
    kept.trim().to_owned()
}

/// The decisions this runner has acted on, by approval and by the proof's key and nonce:
/// a line `approval_id key_id nonce` each, in a file only its user can write.
#[derive(Debug, Default)]
pub(crate) struct Seen {
    path: Option<PathBuf>,
    approvals: HashSet<String>,
    nonces: HashSet<String>,
}

/// The two sets in a seen file's text.
///
/// # Errors
/// A line that is not exactly `approval_id key_id nonce`, or a last line cut short (no
/// new line): a record that cannot be read is never read as "nothing seen".
fn parse_seen(text: &str) -> Result<(HashSet<String>, HashSet<String>), String> {
    if !text.is_empty() && !text.ends_with('\n') {
        return Err("the seen record ends in a cut line".to_owned());
    }
    let (mut approvals, mut nonces) = (HashSet::new(), HashSet::new());
    for (i, line) in text.lines().enumerate() {
        let mut f = line.split_whitespace();
        let (Some(a), Some(k), Some(n), None) = (f.next(), f.next(), f.next(), f.next()) else {
            return Err(format!(
                "the seen record's line {} is not `approval_id key_id nonce`",
                i + 1
            ));
        };
        approvals.insert(a.to_owned());
        nonces.insert(format!("{k} {n}"));
    }
    Ok((approvals, nonces))
}

impl Seen {
    /// The record at `path`; none yet is an empty one.
    ///
    /// # Errors
    /// The file exists and cannot be read, belongs to another user, is writable by group
    /// or world, or has a malformed line.
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        let mut seen = Self {
            path: Some(path.to_owned()),
            ..Self::default()
        };
        if let Some(mut f) = private::open_trusted(path, false)? {
            let mut text = String::new();
            f.read_to_string(&mut text)
                .map_err(|e| format!("cannot read {}: {}", path.display(), e.kind()))?;
            (seen.approvals, seen.nonces) =
                parse_seen(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Ok(seen)
    }

    /// Whether this approval, or its proof's nonce, was acted on before.
    pub(crate) fn contains(&self, approval_id: &str, proof: &Proof) -> bool {
        self.approvals.contains(approval_id)
            || self
                .nonces
                .contains(&format!("{} {}", proof.key_id.trim(), proof.nonce.trim()))
    }

    /// Records it, on disk first, under an exclusive lock on the file: the record is read
    /// again inside the lock, so a second process sharing the file cannot record the
    /// same decision too. `Ok(false)`: it was already there (acted on, here or by
    /// another process), nothing was written.
    ///
    /// # Errors
    /// The record could not be locked, read (or is malformed) or written; the caller then
    /// runs nothing.
    pub(crate) fn record(&mut self, approval_id: &str, proof: &Proof) -> Result<bool, String> {
        let (k, n) = (proof.key_id.trim(), proof.nonce.trim());
        if approval_id.is_empty()
            || approval_id.contains(char::is_whitespace)
            || k.is_empty()
            || n.is_empty()
            || k.contains(char::is_whitespace)
            || n.contains(char::is_whitespace)
        {
            return Err("the approval id, proof key id or nonce cannot be recorded".to_owned());
        }
        if let Some(path) = self.path.clone() {
            let fail =
                |e: &std::io::Error| format!("cannot write {}: {}", path.display(), e.kind());
            let mut f = private::open_trusted(&path, true)?
                .ok_or_else(|| format!("cannot open {}", path.display()))?;
            f.lock().map_err(|e| fail(&e))?;
            let mut text = String::new();
            f.read_to_string(&mut text).map_err(|e| fail(&e))?;
            (self.approvals, self.nonces) =
                parse_seen(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            if self.contains(approval_id, proof) {
                return Ok(false);
            }
            f.write_all(format!("{approval_id} {k} {n}\n").as_bytes())
                .and_then(|()| f.sync_all())
                .map_err(|e| fail(&e))?;
            // The lock goes with the handle.
        } else if self.contains(approval_id, proof) {
            return Ok(false);
        }
        self.approvals.insert(approval_id.to_owned());
        self.nonces.insert(format!("{k} {n}"));
        Ok(true)
    }
}

/// Where this runner keeps one account's approvals state under the CLI's config
/// directory: a directory per account, so two accounts on one machine never share a
/// pinned key or a record of decisions.
///
/// # Errors
/// The account is not made of letters, digits, `_` and `-` (it becomes a path part).
pub(crate) fn state_dir(config_dir: &Path, account: &str) -> Result<PathBuf, String> {
    if account.is_empty()
        || account.len() > 128
        || !account
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err("the account id cannot name a state directory".to_owned());
    }
    Ok(config_dir.join("runner").join("approvals").join(account))
}

/// What the loop needs from the approvals service.
pub(crate) trait RunAsks {
    /// The approved `run` asks of the account (every page, up to a bound).
    async fn approved_runs(&self) -> Result<Vec<Value>, ServiceError>;
    /// Posts one progress report (`AddProgress`).
    async fn report(&self, approval_id: &str, body: &Value) -> Result<(), ServiceError>;
}

/// `apr_` and 26 ULID characters: the only ids put in a path.
fn is_approval_id(s: &str) -> bool {
    s.strip_prefix("apr_").is_some_and(|r| {
        r.len() == 26
            && r.bytes()
                .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())
    })
}

impl RunAsks for HttpService<'_> {
    async fn approved_runs(&self) -> Result<Vec<Value>, ServiceError> {
        let mut all = Vec::new();
        let mut token = String::new();
        for _ in 0..MAX_PAGES {
            let mut q: Vec<(&str, &str)> = vec![
                ("account_id", self.account.as_str()),
                ("status", "approved"),
                ("kind", "run"),
            ];
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

    async fn report(&self, approval_id: &str, body: &Value) -> Result<(), ServiceError> {
        if !is_approval_id(approval_id) {
            return Err(ServiceError::Refused("an approval id that is not one"));
        }
        let mut body = body.clone();
        if let Value::Object(m) = &mut body {
            m.insert("account_id".into(), Value::String(self.account.clone()));
        }
        let path = format!("/v1/approvals/{approval_id}/progress");
        self.api.send_body(Method::Post, &path, &body).await?;
        Ok(())
    }
}

impl HttpService<'_> {
    /// The approvals service's runners (`ListRunners`): id, label, key, revoked or not.
    pub(crate) async fn approval_runners(&self) -> Result<Vec<Value>, ServiceError> {
        let v: Value = self
            .api
            .get(
                "/v1/approvals/runners",
                &[("account_id", self.account.as_str())],
            )
            .await?;
        Ok(v.get("runners")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// Enrols a run key (`EnrollRunner`) with a device key's `proof` over
    /// [`super::signing::run_enroll_message`] and the run key's own `possession`.
    pub(crate) async fn enroll_run_key(
        &self,
        public_key: &str,
        label: &str,
        proof: &Proof,
        possession: &str,
    ) -> Result<Value, ServiceError> {
        let body = json!({
            "account_id": self.account,
            "public_key": public_key,
            "label": label,
            "proof": {
                "key_id": proof.key_id,
                "signed_at": proof.signed_at,
                "nonce": proof.nonce,
                "signature": proof.signature,
            },
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

/// The enrolled, unrevoked runner whose key is `public_key`, from `ListRunners`.
pub(crate) fn enrolled<'v>(runners: &'v [Value], public_key: &str) -> Option<&'v Value> {
    runners
        .iter()
        .find(|r| text(r, "public_key") == public_key && text(r, "revoked_at").is_empty())
}

/// What one poll did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Tick {
    pub(crate) refused: usize,
    pub(crate) done: usize,
    pub(crate) failed: usize,
}

/// The approvals half of the runner.
pub(crate) struct Asks<K> {
    pub(crate) key: RunKey,
    pub(crate) pin: Pin,
    pub(crate) keys: DeviceKeys,
    pub(crate) seen: Seen,
    /// The host's tools (`kubectl` pinned to the configured kube context): a tool that
    /// cannot be used (not installed, or no context configured) fails the action that
    /// needs it, and nothing of that action runs.
    pub(crate) tools: K,
    /// The time a report is signed at, read afresh for each report.
    clock: fn() -> i64,
    /// Approvals handled by this process: never reported twice.
    handled: HashSet<String>,
    /// Approvals recorded in `seen` whose `started` the service has not taken yet: the
    /// next poll sends it again without a second record.
    pending_start: HashSet<String>,
}

/// Unix seconds now.
pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl<K: ArgvRunner> Asks<K> {
    pub(crate) fn new(key: RunKey, pin: Pin, keys: DeviceKeys, seen: Seen, tools: K) -> Self {
        Self::with_clock(key, pin, keys, seen, tools, unix_now)
    }

    /// As [`Asks::new`], on a given clock.
    pub(crate) fn with_clock(
        key: RunKey,
        pin: Pin,
        keys: DeviceKeys,
        seen: Seen,
        tools: K,
        clock: fn() -> i64,
    ) -> Self {
        Self {
            key,
            pin,
            keys,
            seen,
            tools,
            clock,
            handled: HashSet::new(),
            pending_start: HashSet::new(),
        }
    }

    /// A signed report body: `step`, the note as kept, the observed payload for
    /// `started`, and the runner's signature over all of it, at the time now.
    fn report_body(
        &self,
        a: &Approved,
        step: &str,
        note: &str,
        observed: Option<&Value>,
    ) -> Result<Value, String> {
        let now = (self.clock)();
        let note = note_text(note);
        // For `started`, the service hashes the observed payload in its stored form;
        // verify() already held that to `payload_sha256`.
        let seen_hash = if observed.is_some() {
            a.payload_sha256.as_str()
        } else {
            ""
        };
        let message = run_report_message(
            &a.approval_id,
            &a.payload_sha256,
            step,
            &note,
            seen_hash,
            &self.pin.runner_id,
            now,
        );
        let signature = self.key.sign(message.as_bytes())?;
        let mut body = json!({
            "step": step,
            "note": note,
            "runner": {
                "runner_id": self.pin.runner_id,
                "signed_at": now,
                "signature": signature,
            },
        });
        if let Some(run) = observed {
            body["observed"] = json!({ "run": run });
        }
        Ok(body)
    }

    async fn send(
        &self,
        svc: &impl RunAsks,
        a: &Approved,
        step: &str,
        note: &str,
        observed: Option<&Value>,
    ) -> Result<(), String> {
        let body = self.report_body(a, step, note, observed)?;
        svc.report(&a.approval_id, &body)
            .await
            .map_err(|e| format!("the {step} report was not taken: {e}"))
    }

    /// The checks before anything runs; the action to carry out when all pass.
    /// `retry`: the decision was recorded by this process already (its `started` was not
    /// taken), so the record is not a replay.
    fn check(
        &self,
        a: &Approved,
        run: &Value,
        retry: bool,
    ) -> Result<host_action::Action, Refusal> {
        let payload = stored_payload(run)?;
        let now = (self.clock)();
        if payload.get("action").is_none() && payload.get("schema_id").is_none() {
            // A script run: not this runner's to carry out (roll-runner's), whatever it pins.
            verify(a, &payload, &self.pin, &self.keys, now)?;
            return Err(Refusal::NotHostAction("a script run".to_owned()));
        }
        verify(a, &payload, &self.pin, &self.keys, now)?;
        let proof = a.proof.clone().unwrap_or_default();
        if !retry && self.seen.contains(&a.approval_id, &proof) {
            return Err(Refusal::Replay);
        }
        host_action::lookup(&payload["action"]).map_err(Refusal::SchemaInvalid)
    }

    /// One poll: every approved run pinned to this runner is checked, then refused or
    /// carried out, each reported once.
    pub(crate) async fn tick(
        &mut self,
        svc: &impl RunAsks,
        note: &impl Fn(&str),
    ) -> Result<Tick, ServiceError> {
        let mut t = Tick::default();
        for v in svc.approved_runs().await? {
            let a = Approved::from_json(&v);
            let run = v.pointer("/payload/run").cloned().unwrap_or(Value::Null);
            // Another runner's run, or not a run: not ours to report on.
            if text(&run, "runner_id") != self.pin.runner_id
                || self.handled.contains(&a.approval_id)
            {
                continue;
            }
            self.handled.insert(a.approval_id.clone());
            let retry = self.pending_start.contains(&a.approval_id);
            let action = match self.check(&a, &run, retry) {
                Ok(action) => action,
                Err(why) => {
                    t.refused += 1;
                    self.refuse(svc, &a, &why, note).await;
                    continue;
                }
            };
            if !retry {
                // On disk before anything else: a crash after this burns the approval
                // (it is never run twice) rather than risking a second run.
                let proof = a.proof.clone().unwrap_or_default();
                match self.seen.record(&a.approval_id, &proof) {
                    Ok(true) => {}
                    Ok(false) => {
                        t.refused += 1;
                        self.refuse(svc, &a, &Refusal::Replay, note).await;
                        continue;
                    }
                    Err(e) => {
                        t.failed += 1;
                        let said = format!("{e}; nothing was run");
                        note(&format!("approval {}: {said}", a.approval_id));
                        if let Err(e) = self.send(svc, &a, "failed", &said, None).await {
                            note(&format!("approval {}: {e}", a.approval_id));
                        }
                        continue;
                    }
                }
                self.pending_start.insert(a.approval_id.clone());
            }
            if let Err(e) = self.send(svc, &a, "started", "", Some(&run)).await {
                // Not started: nothing runs. A transient refusal is tried at the next poll.
                note(&format!("approval {}: {e}; nothing was run", a.approval_id));
                self.handled.remove(&a.approval_id);
                continue;
            }
            self.pending_start.remove(&a.approval_id);
            // The id only: the parameters are the requester's text.
            note(&format!(
                "approval {}: carrying out {}",
                a.approval_id,
                action.id()
            ));
            let result = host_action::carry_out(&action, &self.tools).await;
            let (step, said) = match result {
                Ok(out) => {
                    t.done += 1;
                    ("done", out)
                }
                Err(e) => {
                    t.failed += 1;
                    ("failed", e)
                }
            };
            note(&format!("approval {}: {step}", a.approval_id));
            if let Err(e) = self.send(svc, &a, step, &said, None).await {
                note(&format!("approval {}: {e}", a.approval_id));
            }
        }
        Ok(t)
    }

    /// Reports a refused run `failed` with the reason.
    async fn refuse(&self, svc: &impl RunAsks, a: &Approved, why: &Refusal, note: &impl Fn(&str)) {
        let said = format!("refused ({}): {why}", why.label());
        note(&format!("approval {}: {said}", a.approval_id));
        if let Err(e) = self.send(svc, a, "failed", &said, None).await {
            note(&format!("approval {}: {e}", a.approval_id));
        }
    }

    /// Polls until `stop` turns true, every [`POLL`], backing off after failures.
    pub(crate) async fn serve(
        &mut self,
        svc: &impl RunAsks,
        mut stop: watch::Receiver<bool>,
        note: &impl Fn(&str),
    ) {
        let mut wait = POLL;
        loop {
            if *stop.borrow() {
                return;
            }
            match self.tick(svc, note).await {
                Ok(_) => wait = POLL,
                Err(e) => {
                    note(&format!("approved runs could not be read: {e}"));
                    wait = (wait * 2).min(POLL_MAX);
                }
            }
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                _ = stop.changed() => {}
            }
        }
    }
}

/// Reads the device keys this runner trusts from `path`; none is an empty set.
///
/// # Errors
/// The file exists and is unreadable or malformed, belongs to another user, or is
/// writable by group or world.
pub(crate) fn load_device_keys(path: &Path) -> Result<DeviceKeys, String> {
    let Some(mut f) = private::open_trusted(path, false)? else {
        return Ok(DeviceKeys::default());
    };
    let mut text = String::new();
    f.read_to_string(&mut text)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e.kind()))?;
    DeviceKeys::parse(&text)
}

/// A device proof as `iohr runner run-key --proof <file>` reads it: the JSON object
/// `{key_id, signed_at, nonce, signature}`.
///
/// # Errors
/// A field is missing.
pub(crate) fn proof_from_json(v: &Value) -> Result<Proof, String> {
    let p = Proof {
        key_id: text(v, "key_id"),
        signed_at: int(v.get("signed_at")),
        nonce: text(v, "nonce"),
        signature: text(v, "signature"),
    };
    if p.key_id.is_empty() || p.signed_at <= 0 || p.nonce.is_empty() || p.signature.is_empty() {
        return Err("the proof needs key_id, signed_at, nonce and signature".to_owned());
    }
    Ok(p)
}

pub(crate) mod enrol;

#[cfg(test)]
mod tests;
