//! The approvals half of the runner: core's vectors (copied in `tests/data/host-action`,
//! see its `SOURCE`), then the loop against a recording service and a recording `kubectl`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use aws_lc_rs::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use serde_json::{Value, json};

use super::*;
use crate::runner::host_action::testing::{Fake, fake};

const VECTORS: [&str; 6] = [
    "bad_signature",
    "expired",
    "hash_mismatch",
    "ok",
    "schema_invalid",
    "wrong_runner",
];

fn data(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/host-action")
        .join(rel)
}

fn vector(name: &str) -> Value {
    let text = std::fs::read_to_string(data(&format!("vectors/{name}.json"))).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn device_keys(v: &Value) -> DeviceKeys {
    let mut keys = DeviceKeys::default();
    for (id, k) in v["device_keys"].as_object().unwrap() {
        keys.insert(id, k.as_str().unwrap()).unwrap();
    }
    keys
}

fn verdict_of(v: &Value) -> String {
    let pin = Pin {
        runner_id: v["expect"]["runner_id"].as_str().unwrap().to_owned(),
        runner_public_key: v["expect"]["runner_public_key"]
            .as_str()
            .unwrap()
            .to_owned(),
    };
    let a = Approved::from_json(&v["approval"]);
    match verify(
        &a,
        &v["approval"]["payload"],
        &pin,
        &device_keys(v),
        v["now"].as_i64().unwrap(),
    ) {
        Ok(()) => "ok".to_owned(),
        Err(r) => r.label().to_owned(),
    }
}

#[test]
fn every_vector_gets_core_s_verdict() {
    for name in VECTORS {
        let v = vector(name);
        assert_eq!(v["name"], name);
        assert_eq!(verdict_of(&v), v["verdict"].as_str().unwrap(), "{name}");
    }
}

#[test]
fn every_copied_vector_is_checked() {
    let mut found: Vec<String> = std::fs::read_dir(data("vectors"))
        .unwrap()
        .map(|e| {
            e.unwrap()
                .path()
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    found.sort();
    assert_eq!(found, VECTORS);
}

#[test]
fn the_decision_message_is_core_s_bytes() {
    for name in VECTORS {
        let v = vector(name);
        let (Some(want), Some(proof)) = (
            v["message"].as_str(),
            Approved::from_json(&v["approval"]).proof,
        ) else {
            continue;
        };
        let a = Approved::from_json(&v["approval"]);
        let got = decision_message(
            &a.approval_id,
            &a.payload_sha256,
            "approve",
            &a.decision_note,
            proof.signed_at,
            &proof.nonce,
            &proof.key_id,
        );
        assert_eq!(got, want, "{name}");
    }
}

#[test]
fn the_ok_vector_hashes_as_recorded() {
    let v = vector("ok");
    let payload = &v["approval"]["payload"];
    assert_eq!(
        payload_digest(payload),
        v["approval"]["payload_sha256"].as_str().unwrap()
    );
    assert_eq!(
        payload_digest(&payload["action"]),
        payload["script_sha256"].as_str().unwrap()
    );
}

/// A stored payload as REST sends it: 64-bit integers as strings, every field present,
/// the action as JSON text.
fn rest_run(stored: &Value) -> Value {
    json!({
        "cmd": stored["cmd"],
        "script_sha256": stored["script_sha256"],
        "argv": stored["argv"],
        "plan_sha256": "",
        "effect": stored["effect"],
        "runner_id": stored["runner_id"],
        "runner_public_key": stored["runner_public_key"],
        "expires_at": stored["expires_at"].as_i64().unwrap().to_string(),
        "version": 0,
        "nonce": "",
        "schema_id": stored.get("schema_id").cloned().unwrap_or(json!("")),
        "action": stored.get("action").map(Value::to_string).unwrap_or_default(),
    })
}

#[test]
fn a_rest_run_becomes_the_form_it_was_hashed_in() {
    let v = vector("ok");
    let stored = &v["approval"]["payload"];
    let back = stored_payload(&rest_run(stored)).unwrap();
    assert_eq!(&back, stored);
    assert_eq!(
        payload_digest(&back),
        v["approval"]["payload_sha256"].as_str().unwrap()
    );
}

#[test]
fn a_roll_train_is_not_carried_out() {
    let v = vector("ok");
    let mut rest = rest_run(&v["approval"]["payload"]);
    rest["roll"] = json!({ "stages": [] });
    assert_eq!(
        stored_payload(&rest).unwrap_err().label(),
        "not_host_action"
    );
}

#[test]
fn device_keys_read_from_a_file_skip_comments_and_refuse_junk() {
    let v = vector("ok");
    let key = v["device_keys"]["device-0"].as_str().unwrap();
    let keys = DeviceKeys::parse(&format!("# the owner's phone\n\ndevice-0 {key}\n")).unwrap();
    assert_eq!(keys.len(), 1);
    assert!(DeviceKeys::parse("device-0").is_err());
    assert!(DeviceKeys::parse("device-0 AAAA").is_err());
    assert!(DeviceKeys::parse(&format!("device-0 {key} extra")).is_err());
}

#[test]
fn a_note_is_kept_as_the_service_keeps_it() {
    assert_eq!(note_text("  a\u{7}b\n\tc  "), "ab\n\tc");
    assert_eq!(note_text(&"x".repeat(900)).chars().count(), NOTE_MAX);
}

// The loop.

const RUNNER: &str = "rnr_00000000000000000000000001";
const NOW: i64 = 1_893_452_400;

struct World {
    device: RunKey,
    run: RunKey,
    /// The run key's PKCS#8: the loop takes a key of its own.
    run_doc: Vec<u8>,
}

fn world() -> World {
    let (run, doc) = RunKey::generate().unwrap();
    World {
        device: RunKey::generate().unwrap().0,
        run,
        run_doc: doc.to_vec(),
    }
}

fn action(params: &Value) -> Value {
    json!({
        "action": "k8s.delete-failed-pods",
        "runner_id": RUNNER,
        "params": params,
        "effect": "Deletes every failed pod in namespace example.",
    })
}

struct Ask<'a> {
    id: &'a str,
    nonce: &'a str,
    action: Value,
    expires_at: i64,
    runner_id: &'a str,
    runner_key: Option<String>,
    signer: Option<&'a RunKey>,
}

/// A random nonce per name and test thread: the same name gives the same nonce within one test
/// (a replay, or a proof matching its ask), and nothing hard-coded reaches the signed payload.
fn nonce(name: &str) -> &'static str {
    thread_local! {
        static NONCES: RefCell<HashMap<String, &'static str>> = RefCell::default();
    }
    NONCES.with(|n| {
        *n.borrow_mut().entry(name.to_owned()).or_insert_with(|| {
            let mut b = [0u8; 8];
            getrandom::fill(&mut b).unwrap();
            // Leaked so an `Ask` can borrow it for the whole test; a few bytes per test.
            Box::leak(format!("{:016x}", u64::from_le_bytes(b)).into_boxed_str())
        })
    })
}

fn ask<'a>(id: &'a str, nonce: &'a str) -> Ask<'a> {
    Ask {
        id,
        nonce,
        action: action(&json!({ "namespace": "example" })),
        expires_at: NOW + 3600,
        runner_id: RUNNER,
        runner_key: None,
        signer: None,
    }
}

/// An approved run as `GET /v1/approvals` returns it, decided with the world's device
/// key (or `signer`).
fn approval(w: &World, a: Ask<'_>) -> Value {
    let id_word = a.action["action"].as_str().unwrap().to_owned();
    let stored = json!({
        "cmd": id_word,
        "argv": [id_word],
        "script_sha256": payload_digest(&a.action),
        "effect": "remote",
        "runner_id": a.runner_id,
        "runner_public_key": a.runner_key.unwrap_or_else(|| w.run.public_key()),
        "expires_at": a.expires_at,
        "schema_id": HOST_ACTION,
        "action": a.action,
    });
    let hash = payload_digest(&stored);
    let message = decision_message(a.id, &hash, "approve", "", 1, a.nonce, "device-0");
    let signature = a
        .signer
        .unwrap_or(&w.device)
        .sign(message.as_bytes())
        .unwrap();
    json!({
        "approval_id": a.id,
        "kind": "run",
        "status": "approved",
        "decided_by": "person-0",
        "decision_note": "",
        "payload_sha256": hash,
        "decision_proof": { "key_id": "device-0", "signed_at": "1", "nonce": a.nonce, "signature": signature },
        "payload": { "run": rest_run(&stored) },
    })
}

#[derive(Default)]
struct Mock {
    approvals: Vec<Value>,
    reports: Mutex<Vec<(String, Value)>>,
    refuse_started: Mutex<usize>,
}

#[allow(
    unknown_lints,
    clippy::unused_async_trait_impl,
    reason = "a fake answers at once; the trait is async for the service"
)]
impl RunAsks for Mock {
    async fn approved_runs(&self) -> Result<Vec<Value>, ServiceError> {
        Ok(self.approvals.clone())
    }

    async fn report(&self, approval_id: &str, body: &Value) -> Result<(), ServiceError> {
        let mut refuse = self.refuse_started.lock().unwrap();
        if body["step"] == "started" && *refuse > 0 {
            *refuse -= 1;
            return Err(ServiceError::Refused("the service is busy"));
        }
        self.reports
            .lock()
            .unwrap()
            .push((approval_id.to_owned(), body.clone()));
        Ok(())
    }
}

impl Mock {
    fn with(approvals: Vec<Value>) -> Self {
        Self {
            approvals,
            ..Self::default()
        }
    }

    fn steps(&self) -> Vec<(String, String)> {
        self.reports
            .lock()
            .unwrap()
            .iter()
            .map(|(id, b)| (id.clone(), b["step"].as_str().unwrap().to_owned()))
            .collect()
    }

    fn note(&self, i: usize) -> String {
        self.reports.lock().unwrap()[i].1["note"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}

fn asks(w: &World, seen: Seen, kubectl: Option<Fake>) -> Asks<Fake> {
    let mut keys = DeviceKeys::default();
    keys.insert("device-0", &w.device.public_key()).unwrap();
    let pin = Pin {
        runner_id: RUNNER.to_owned(),
        runner_public_key: w.run.public_key(),
    };
    let key = RunKey::from_pkcs8(&w.run_doc).unwrap();
    let tools = kubectl.unwrap_or_else(|| {
        let mut f = fake("", false);
        f.missing = Some("kubectl is not on this runner's PATH".to_owned());
        f
    });
    Asks::with_clock(key, pin, keys, seen, tools, || NOW)
}

/// Whether `body` is signed by the world's run key over core's `run_report` message.
fn signed(w: &World, approval: &Value, body: &Value) -> bool {
    let step = body["step"].as_str().unwrap();
    let hash = approval["payload_sha256"].as_str().unwrap();
    let observed = if step == "started" { hash } else { "" };
    let message = run_report_message(
        approval["approval_id"].as_str().unwrap(),
        hash,
        step,
        body["note"].as_str().unwrap(),
        observed,
        body["runner"]["runner_id"].as_str().unwrap(),
        body["runner"]["signed_at"].as_i64().unwrap(),
    );
    let point = B64URL.decode(w.run.public_key()).unwrap();
    let sig = B64URL
        .decode(body["runner"]["signature"].as_str().unwrap())
        .unwrap();
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, &point)
        .verify(message.as_bytes(), &sig)
        .is_ok()
}

const ID1: &str = "apr_00000000000000000000000001";
const ID2: &str = "apr_00000000000000000000000002";

#[tokio::test]
async fn an_approved_action_is_started_carried_out_and_done() {
    let w = world();
    let a = approval(&w, ask(ID1, nonce("nonce-1")));
    let svc = Mock::with(vec![a.clone()]);
    let mut asks = asks(&w, Seen::default(), Some(fake("", false)));
    let t = asks.tick(&svc, &|_| {}).await.unwrap();
    assert_eq!(
        t,
        Tick {
            refused: 0,
            done: 1,
            failed: 0
        }
    );
    assert_eq!(
        svc.steps(),
        [(ID1.into(), "started".into()), (ID1.into(), "done".into())]
    );
    let reports = svc.reports.lock().unwrap().clone();
    for (_, body) in &reports {
        assert!(signed(&w, &a, body), "{body}");
        assert_eq!(body["runner"]["runner_id"], RUNNER);
    }
    // `started` sends back the payload as it was read; the service hashes it as recorded.
    let observed = &reports[0].1["observed"]["run"];
    assert_eq!(observed, &a["payload"]["run"]);
    assert_eq!(
        payload_digest(&stored_payload(observed).unwrap()),
        a["payload_sha256"].as_str().unwrap()
    );
    assert!(reports[1].1.get("observed").is_none());
    let calls = asks.tools.calls();
    assert_eq!(
        calls,
        [(
            host_action::Tool::Kubectl,
            host_action::delete_failed_pods_argv("example").unwrap()
        )]
    );
    // A second poll that still lists it reports nothing more.
    asks.tick(&svc, &|_| {}).await.unwrap();
    assert_eq!(svc.steps().len(), 2);
}

#[tokio::test]
async fn a_replayed_nonce_runs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_dir(dir.path(), "acct").unwrap().join("seen");
    let w = world();
    let first = Mock::with(vec![approval(&w, ask(ID1, nonce("nonce-1")))]);
    let mut asks1 = asks(&w, Seen::load(&path).unwrap(), Some(fake("", false)));
    asks1.tick(&first, &|_| {}).await.unwrap();
    assert_eq!(first.steps().last().unwrap().1, "done");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // A new process: the same decision under another approval, and the same approval.
    let again = Mock::with(vec![
        approval(&w, ask(ID2, nonce("nonce-1"))),
        approval(&w, ask(ID1, nonce("nonce-1"))),
    ]);
    let mut asks2 = asks(&w, Seen::load(&path).unwrap(), Some(fake("", false)));
    let t = asks2.tick(&again, &|_| {}).await.unwrap();
    assert_eq!(t.refused, 2);
    assert_eq!(
        again.steps(),
        [(ID2.into(), "failed".into()), (ID1.into(), "failed".into())]
    );
    assert!(again.note(0).starts_with("refused (replay)"));
    assert!(asks2.tools.ran_nothing());
}

async fn refused_with(w: &World, a: Value, label: &str) {
    let svc = Mock::with(vec![a]);
    let mut asks = asks(w, Seen::default(), Some(fake("default", false)));
    let t = asks.tick(&svc, &|_| {}).await.unwrap();
    assert_eq!(t.refused, 1, "{label}");
    assert_eq!(svc.steps(), [(ID1.into(), "failed".into())], "{label}");
    let note = svc.note(0);
    assert!(note.starts_with(&format!("refused ({label})")), "{note}");
    let body = svc.reports.lock().unwrap()[0].1.clone();
    assert!(signed(w, &svc.approvals[0], &body));
    assert!(asks.tools.ran_nothing(), "{label}: something ran");
}

#[tokio::test]
async fn an_unknown_action_is_refused_and_nothing_runs() {
    let w = world();
    let mut a = ask(ID1, nonce("nonce-1"));
    a.action["action"] = json!("k8s.reboot-node");
    let svc = Mock::with(vec![approval(&w, a)]);
    let mut asks = asks(&w, Seen::default(), Some(fake("", false)));
    asks.tick(&svc, &|_| {}).await.unwrap();
    assert_eq!(svc.steps(), [(ID1.into(), "failed".into())]);
    assert!(svc.note(0).contains("unknown action"), "{}", svc.note(0));
    assert!(asks.tools.ran_nothing());
}

#[tokio::test]
async fn a_cluster_namespace_is_refused_before_anything_starts() {
    let w = world();
    for ns in host_action::DENIED_NAMESPACES {
        let mut a = ask(ID1, nonce("nonce-1"));
        a.action = action(&json!({ "namespace": ns }));
        refused_with(&w, approval(&w, a), "schema_invalid").await;
    }
}

#[tokio::test]
async fn each_refusal_reports_failed_and_runs_nothing() {
    let w = world();
    let mut expired = ask(ID1, nonce("nonce-1"));
    expired.expires_at = NOW - 1;
    refused_with(&w, approval(&w, expired), "expired").await;

    let mut other_key = ask(ID1, nonce("nonce-1"));
    other_key.runner_key = Some(world().run.public_key());
    refused_with(&w, approval(&w, other_key), "wrong_runner").await;

    let stranger = world();
    let mut forged = ask(ID1, nonce("nonce-1"));
    forged.signer = Some(&stranger.device);
    refused_with(&w, approval(&w, forged), "bad_signature").await;

    let mut tampered = approval(&w, ask(ID1, nonce("nonce-1")));
    tampered["payload"]["run"]["effect"] = json!("local");
    refused_with(&w, tampered, "hash_mismatch").await;

    let mut by_service = approval(&w, ask(ID1, nonce("nonce-1")));
    by_service["decided_by"] = json!("svc:auto");
    refused_with(&w, by_service, "no_approver").await;

    let mut unsigned = approval(&w, ask(ID1, nonce("nonce-1")));
    unsigned["decision_proof"] = Value::Null;
    refused_with(&w, unsigned, "bad_signature").await;
}

#[tokio::test]
async fn another_runner_s_run_is_left_alone() {
    let w = world();
    let mut a = ask(ID1, nonce("nonce-1"));
    a.runner_id = "rnr_00000000000000000000000009";
    a.action["runner_id"] = json!("rnr_00000000000000000000000009");
    let svc = Mock::with(vec![approval(&w, a)]);
    let mut asks = asks(&w, Seen::default(), Some(fake("", false)));
    let t = asks.tick(&svc, &|_| {}).await.unwrap();
    assert_eq!(t, Tick::default());
    assert_eq!(svc.steps(), Vec::<(String, String)>::new());
}

#[tokio::test]
async fn a_start_the_service_did_not_take_runs_nothing_and_is_tried_again() {
    let w = world();
    let svc = Mock::with(vec![approval(&w, ask(ID1, nonce("nonce-1")))]);
    *svc.refuse_started.lock().unwrap() = 1;
    let mut asks = asks(&w, Seen::default(), Some(fake("", false)));
    asks.tick(&svc, &|_| {}).await.unwrap();
    assert_eq!(svc.steps(), Vec::<(String, String)>::new());
    assert!(asks.tools.ran_nothing());
    asks.tick(&svc, &|_| {}).await.unwrap();
    assert_eq!(
        svc.steps(),
        [(ID1.into(), "started".into()), (ID1.into(), "done".into())]
    );
}

#[tokio::test]
async fn a_refused_delete_reports_failed_with_kubectl_s_words() {
    let w = world();
    let a = approval(&w, ask(ID1, nonce("nonce-1")));
    let svc = Mock::with(vec![a.clone()]);
    let mut asks = asks(&w, Seen::default(), Some(fake("", true)));
    let t = asks.tick(&svc, &|_| {}).await.unwrap();
    assert_eq!(t.failed, 1);
    assert_eq!(
        svc.steps(),
        [
            (ID1.into(), "started".into()),
            (ID1.into(), "failed".into())
        ]
    );
    assert_eq!(svc.note(1), "example: forbidden");
    assert!(signed(&w, &a, &svc.reports.lock().unwrap()[1].1));
}

#[tokio::test]
async fn no_kubectl_reports_failed() {
    let w = world();
    let svc = Mock::with(vec![approval(&w, ask(ID1, nonce("nonce-1")))]);
    let mut asks = asks(&w, Seen::default(), None);
    asks.tick(&svc, &|_| {}).await.unwrap();
    assert_eq!(svc.steps()[1].1, "failed");
    assert!(svc.note(1).contains("kubectl is not on this runner's PATH"));
}

#[test]
fn a_seen_record_refuses_what_it_cannot_write_back() {
    let mut seen = Seen::default();
    let proof = Proof {
        key_id: "device 0".to_owned(),
        nonce: "n".to_owned(),
        ..Proof::default()
    };
    assert!(seen.record(ID1, &proof).is_err());
}

#[test]
fn the_run_key_signs_what_its_public_key_verifies() {
    let (key, doc) = RunKey::generate().unwrap();
    let again = RunKey::from_pkcs8(&doc).unwrap();
    assert_eq!(key.public_key(), again.public_key());
    let sig = B64URL.decode(key.sign(b"message").unwrap()).unwrap();
    let point = B64URL.decode(key.public_key()).unwrap();
    assert_eq!(point.len(), 65);
    assert!(
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, &point)
            .verify(b"message", &sig)
            .is_ok()
    );
    let shown = format!("{key:?}");
    assert!(shown.contains(&key.public_key()));
    assert!(!shown.contains(&B64URL.encode(doc.as_slice())));
}

// Items of the review of the approvals executor: what is trusted, and how.

fn seen_lines(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn proof(nonce: &str) -> Proof {
    Proof {
        key_id: "device-0".to_owned(),
        nonce: nonce.to_owned(),
        ..Proof::default()
    }
}

#[test]
fn the_state_directory_is_one_per_account_and_never_a_path() {
    let base = std::path::Path::new("/cfg");
    let a = state_dir(base, "acc_1").unwrap();
    let b = state_dir(base, "acc_2").unwrap();
    assert_ne!(a, b);
    assert_eq!(a, base.join("runner/approvals/acc_1"));
    for bad in ["", "..", "a/b", "../x", "a b", "a\\b", "é"] {
        assert!(state_dir(base, bad).is_err(), "{bad:?}");
    }
}

#[test]
fn a_malformed_seen_line_is_an_error_not_a_skip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("seen");
    let ok = format!("{ID1} device-0 n1\n");
    for (text, why) in [
        ("garbage\n", "one field"),
        ("a b\n", "two fields"),
        ("a b c d\n", "four fields"),
        ("a b c\nbroken line\n", "a later bad line"),
        ("a b c", "a cut last line"),
    ] {
        std::fs::write(&path, text).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let err = Seen::load(&path).unwrap_err();
        assert!(err.contains("seen record"), "{why}: {err}");
    }
    std::fs::write(&path, &ok).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(
        Seen::load(&path)
            .unwrap()
            .contains(ID1, &proof(nonce("other")))
    );
}

#[test]
fn recording_into_a_malformed_seen_file_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("seen");
    let mut seen = Seen::load(&path).unwrap();
    std::fs::write(&path, "garbage\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(seen.record(ID1, &proof(nonce("n1"))).is_err());
    assert_eq!(seen_lines(&path), ["garbage"]);
}

#[cfg(unix)]
#[test]
fn the_trust_root_and_the_seen_file_must_be_the_user_s_and_not_writable_by_others() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let w = world();
    let keys = dir.path().join("device-keys");
    let seen = dir.path().join("seen");
    std::fs::write(&keys, format!("device-0 {}\n", w.device.public_key())).unwrap();
    std::fs::write(&seen, "").unwrap();
    let mode = |p: &std::path::Path, m: u32| {
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
    };
    // Owner-only and owner-writable-only files load.
    for m in [0o600, 0o644, 0o400] {
        mode(&keys, m);
        mode(&seen, m);
        assert_eq!(load_device_keys(&keys).unwrap().len(), 1, "{m:o}");
        assert!(Seen::load(&seen).is_ok(), "{m:o}");
    }
    // Group- or world-writable is refused, whatever else is right.
    for m in [0o620, 0o602, 0o660, 0o666, 0o777] {
        mode(&keys, m);
        mode(&seen, m);
        let e = load_device_keys(&keys).unwrap_err();
        assert!(e.contains("writable by group or world"), "{m:o}: {e}");
        let e = Seen::load(&seen).unwrap_err();
        assert!(e.contains("writable by group or world"), "{m:o}: {e}");
        // And a record is not written into it.
        let mut s = Seen {
            path: Some(seen.clone()),
            ..Seen::default()
        };
        assert!(s.record(ID1, &proof(nonce("n1"))).is_err());
        assert_eq!(std::fs::read_to_string(&seen).unwrap(), "");
    }
    // A file another user owns is refused.
    let e = load_device_keys(std::path::Path::new("/etc/hosts")).unwrap_err();
    assert!(e.contains("owned by another user"), "{e}");
    let e = Seen::load(std::path::Path::new("/etc/hosts")).unwrap_err();
    assert!(e.contains("owned by another user"), "{e}");
    // A writable directory above it is refused too.
    mode(&keys, 0o600);
    mode(dir.path(), 0o777);
    let e = load_device_keys(&keys).unwrap_err();
    assert!(e.contains("writable by group or world"), "{e}");
    mode(dir.path(), 0o700);
}

#[tokio::test]
async fn the_decision_is_recorded_before_started_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_dir(dir.path(), "acct").unwrap().join("seen");
    let w = world();
    let svc = Mock::with(vec![approval(&w, ask(ID1, nonce("nonce-1")))]);
    *svc.refuse_started.lock().unwrap() = 1;
    let mut asks = asks(&w, Seen::load(&path).unwrap(), Some(fake("", false)));
    asks.tick(&svc, &|_| {}).await.unwrap();
    // The service took no start, nothing ran, and the decision is already on disk.
    assert_eq!(svc.steps(), Vec::<(String, String)>::new());
    assert_eq!(seen_lines(&path).len(), 1);
    assert!(seen_lines(&path)[0].starts_with(ID1));
    // The retry is not a replay of its own record: it starts, runs and finishes once.
    asks.tick(&svc, &|_| {}).await.unwrap();
    assert_eq!(
        svc.steps(),
        [(ID1.into(), "started".into()), (ID1.into(), "done".into())]
    );
    assert_eq!(seen_lines(&path).len(), 1);
    assert_eq!(asks.tools.calls().len(), 1);
    // A new process after a restart finds it recorded and runs nothing.
    let svc2 = Mock::with(vec![approval(&w, ask(ID1, nonce("nonce-1")))]);
    let mut asks2 = asks_with(&w, &path);
    let t = asks2.tick(&svc2, &|_| {}).await.unwrap();
    assert_eq!(t.refused, 1);
    assert!(asks2.tools.ran_nothing());
}

fn asks_with(w: &World, path: &std::path::Path) -> Asks<Fake> {
    asks(w, Seen::load(path).unwrap(), Some(fake("", false)))
}

#[tokio::test]
async fn a_record_that_cannot_be_written_runs_nothing_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("seen");
    let w = world();
    let mut seen = Seen::load(&path).unwrap();
    // The record is replaced by a directory: it cannot be opened for writing.
    std::fs::create_dir(&path).unwrap();
    seen.path = Some(path.clone());
    let svc = Mock::with(vec![approval(&w, ask(ID1, nonce("nonce-1")))]);
    let mut asks = asks(&w, seen, Some(fake("", false)));
    let t = asks.tick(&svc, &|_| {}).await.unwrap();
    assert_eq!(t.failed, 1);
    assert_eq!(svc.steps(), [(ID1.into(), "failed".into())]);
    assert!(svc.note(0).contains("nothing was run"));
    assert!(asks.tools.ran_nothing());
}

#[test]
fn two_handles_on_one_seen_file_record_a_decision_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("seen");
    let mut one = Seen::load(&path).unwrap();
    let mut two = Seen::load(&path).unwrap();
    assert_eq!(one.record(ID1, &proof(nonce("n1"))), Ok(true));
    // `two` loaded before the write and does not know it; the file says so under the lock.
    assert_eq!(two.record(ID2, &proof(nonce("n1"))), Ok(false));
    assert_eq!(two.record(ID1, &proof(nonce("n9"))), Ok(false));
    assert_eq!(seen_lines(&path).len(), 1);
    assert_eq!(two.record(ID2, &proof(nonce("n2"))), Ok(true));
    assert_eq!(seen_lines(&path).len(), 2);
}

#[tokio::test]
async fn a_replay_the_load_missed_is_caught_under_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("seen");
    let w = world();
    // Loaded empty; another process records the same decision before this one acts.
    let seen = Seen::load(&path).unwrap();
    Seen::load(&path)
        .unwrap()
        .record(ID2, &proof(nonce("nonce-1")))
        .unwrap();
    let svc = Mock::with(vec![approval(&w, ask(ID1, nonce("nonce-1")))]);
    let mut asks = asks(&w, seen, Some(fake("", false)));
    let t = asks.tick(&svc, &|_| {}).await.unwrap();
    assert_eq!(t.refused, 1);
    assert_eq!(svc.steps(), [(ID1.into(), "failed".into())]);
    assert!(svc.note(0).starts_with("refused (replay)"));
    assert!(asks.tools.ran_nothing());
}

#[tokio::test]
async fn an_unconfigured_cluster_runs_nothing() {
    let w = world();
    let svc = Mock::with(vec![approval(&w, ask(ID1, nonce("nonce-1")))]);
    let mut asks = asks(&w, Seen::default(), None);
    asks.tools.missing = Some(host_action::KubeTarget::new(None, None).unwrap_err());
    asks.tick(&svc, &|_| {}).await.unwrap();
    assert_eq!(svc.steps().last().unwrap().1, "failed");
    assert!(svc.note(1).contains("no kube context is configured"));
}

#[tokio::test]
async fn each_report_is_signed_at_the_time_it_is_made() {
    use std::sync::atomic::{AtomicI64, Ordering};
    static CLOCK: AtomicI64 = AtomicI64::new(NOW);
    let w = world();
    let svc = Mock::with(vec![approval(&w, ask(ID1, nonce("nonce-1")))]);
    let mut asks = asks(&w, Seen::default(), Some(fake("", false)));
    asks.clock = || CLOCK.fetch_add(10, Ordering::SeqCst);
    asks.tick(&svc, &|_| {}).await.unwrap();
    let times: Vec<i64> = svc
        .reports
        .lock()
        .unwrap()
        .iter()
        .map(|(_, b)| b["runner"]["signed_at"].as_i64().unwrap())
        .collect();
    assert_eq!(times.len(), 2);
    assert!(times[0] < times[1], "{times:?}");
}
