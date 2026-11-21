//! The record, the guard and the agent (RFC 0230 slice 1). The abuse cases are the threat
//! model's (docs/security/p2p-threat-model.md): each refusal has a case here.

use std::path::PathBuf;
use std::time::Duration;

use proptest::prelude::*;

use super::agent::{self, Bootstrap, Phase, Registered, State, Timing};
use super::guard::{Freshness, Guard, Limits, Refusal};
use super::record::{self, Kind, Reason, Report, normalise};
use crate::mac_sign::{DeviceSigner, fresh_nonce};
use crate::runner::approvals::DeviceKeys;
use crate::runner::signing::RunKey;

/// 2026-11-12T23:20:00Z.
const NOW: i64 = 1_794_000_000;
const HOUR: i64 = 3600;
const DAY: i64 = 24 * HOUR;

fn key() -> RunKey {
    RunKey::generate().expect("a key").0
}

fn keys(list: &[(&str, &RunKey)]) -> DeviceKeys {
    let mut k = DeviceKeys::default();
    for (id, key) in list {
        k.insert(id, &key.public_key()).expect("a public key");
    }
    k
}

fn domain_report(k: &RunKey, key_id: &str, subject: &str, at: i64) -> Report {
    Report::sign(k, key_id, Kind::Domain, subject, Reason::FakeShop, at).expect("signed")
}

/// A report signed over a chosen nonce (what a replaying peer could send).
fn with_nonce(k: &RunKey, key_id: &str, subject: &str, at: i64, nonce: &str) -> Report {
    let m = record::message(Kind::Domain, subject, Reason::Phishing, at, nonce, key_id);
    Report {
        v: record::VERSION,
        kind: Kind::Domain,
        subject: subject.to_owned(),
        reason: Reason::Phishing,
        signed_at: at,
        nonce: nonce.to_owned(),
        key_id: key_id.to_owned(),
        signature: k.sign(m.as_bytes()).expect("signed"),
    }
}

// The record.

#[test]
fn p2p_subjects_are_normalised_or_refused() {
    let ok = [
        (Kind::Domain, "Shop.Example.COM.", "shop.example.com"),
        (Kind::Domain, "  xn--bcher-kva.de ", "xn--bcher-kva.de"),
        (Kind::Domain, "a-b.c-d.example", "a-b.c-d.example"),
        (Kind::Handle, "Telegram/@Shop_Help", "telegram/@shop_help"),
        (Kind::Handle, "x/@a.b-c_1", "x/@a.b-c_1"),
    ];
    for (kind, raw, want) in ok {
        assert_eq!(normalise(kind, raw).as_deref(), Ok(want), "{raw}");
    }
    let bad = [
        (Kind::Domain, ""),
        (Kind::Domain, "localhost"),
        (Kind::Domain, "1.2.3.4"),
        (Kind::Domain, "bücher.de"),
        (Kind::Domain, "under_score.com"),
        (Kind::Domain, "-lead.com"),
        (Kind::Domain, "trail-.com"),
        (Kind::Domain, "two..dots.com"),
        (Kind::Domain, "sp ace.com"),
        (Kind::Domain, "https://shop.example"),
        (Kind::Domain, "shop.example/path"),
        (Kind::Handle, "myspace/@someone"),
        (Kind::Handle, "telegram/someone"),
        (Kind::Handle, "telegram/@"),
        (Kind::Handle, "telegram/@two words"),
        (Kind::Handle, "telegram/@someone@else"),
        (Kind::Handle, "person@example.com"),
        (Kind::Handle, "+385911234567"),
    ];
    for (kind, raw) in bad {
        assert!(normalise(kind, raw).is_err(), "{raw} was accepted");
    }
    let long = format!("{}.com", "a".repeat(64));
    assert!(normalise(Kind::Domain, &long).is_err());
    let long = format!("telegram/@{}", "a".repeat(65));
    assert!(normalise(Kind::Handle, &long).is_err());
}

#[test]
fn p2p_errors_never_repeat_the_subject() {
    // A subject is a claim about someone: it must not reach a log through an error.
    let marker = "zq9marker";
    let mut errors = vec![
        normalise(Kind::Domain, marker).expect_err("one label"),
        normalise(Kind::Domain, &format!("{marker}_x.com")).expect_err("underscore"),
        normalise(Kind::Domain, &format!("{marker}.123")).expect_err("numeric tld"),
        normalise(Kind::Handle, &format!("nowhere/@{marker}")).expect_err("network"),
        normalise(Kind::Handle, &format!("telegram/@{marker}!")).expect_err("character"),
        Reason::parse(marker).expect_err("reason"),
    ];
    let k = key();
    errors.push(
        Report::sign(
            &k,
            "apk_1",
            Kind::Domain,
            &format!("{marker}!.com"),
            Reason::Malware,
            NOW,
        )
        .expect_err("bad subject"),
    );
    let wire = format!(
        r#"{{"v":1,"kind":"domain","subject":"{marker}","reason":"phishing","signed_at":{NOW},"nonce":"AAAAAAAAAAAAAAAAAAAAAA","key_id":"apk_1","signature":"AA"}}"#
    );
    errors.push(Report::parse(wire.as_bytes()).expect_err("not normal"));
    errors.push(Report::parse(format!("{{\"{marker}\":1}}").as_bytes()).expect_err("field"));
    for e in errors {
        assert!(!e.contains(marker), "an error repeats the subject: {e}");
    }
}

#[test]
fn p2p_message_and_id_are_a_fixed_vector() {
    // Other languages (eye's client) sign the same bytes: this vector is the contract.
    let m = record::message(
        Kind::Domain,
        "shop.example",
        Reason::FakeShop,
        NOW,
        "AAAAAAAAAAAAAAAAAAAAAA",
        "apk_01TEST",
    );
    assert_eq!(
        m,
        "iohr.p2p.v1.Report\n1\ndomain\nshop.example\nfake_shop\n1794000000\nAAAAAAAAAAAAAAAAAAAAAA\napk_01TEST"
    );
    let r = Report {
        v: 1,
        kind: Kind::Domain,
        subject: "shop.example".into(),
        reason: Reason::FakeShop,
        signed_at: NOW,
        nonce: "AAAAAAAAAAAAAAAAAAAAAA".into(),
        key_id: "apk_01TEST".into(),
        signature: "AA".into(),
    };
    assert_eq!(
        r.id(),
        "fd8bbdfef04af5cae017cc76be7366803aa9c3b4802e3d9d800c9c0fb0b50d4e"
    );
}

#[test]
fn p2p_a_signed_report_travels_and_verifies() {
    let k = key();
    let other = key();
    let r = Report::sign(
        &k,
        "apk_a",
        Kind::Handle,
        "Instagram/@Shop.Deals",
        Reason::Impersonation,
        NOW,
    )
    .expect("signed");
    assert_eq!(r.subject, "instagram/@shop.deals");
    let wire = serde_json::to_vec(&r).expect("json");
    assert!(wire.len() < 400, "a report is {} bytes", wire.len());
    let back = Report::parse(&wire).expect("parsed");
    assert_eq!(back, r);
    assert_eq!(back.id(), r.id());
    assert!(back.verifies(&keys(&[("apk_a", &k)])));
    // Another key under the same id, or no key at all: no.
    assert!(!back.verifies(&keys(&[("apk_a", &other)])));
    assert!(!back.verifies(&keys(&[("apk_b", &k)])));
    assert!(!back.verifies(&DeviceKeys::default()));
    // RunKey is the software DeviceSigner: its public key is the one DeviceKeys holds.
    assert_eq!(
        DeviceSigner::public_key(&k).expect("public"),
        k.public_key()
    );
}

#[test]
fn p2p_records_with_extra_fields_or_too_large_are_refused() {
    let k = key();
    let r = domain_report(&k, "apk_a", "shop.example", NOW);
    let mut v = serde_json::to_value(&r).expect("json");
    v["note"] = serde_json::json!("free text travels here");
    assert!(Report::parse(&serde_json::to_vec(&v).expect("json")).is_err());
    let mut v = serde_json::to_value(&r).expect("json");
    v["v"] = serde_json::json!(2);
    assert!(Report::parse(&serde_json::to_vec(&v).expect("json")).is_err());
    let big = vec![b' '; record::MAX_RECORD + 1];
    assert!(Report::parse(&big).is_err());
}

#[derive(Debug, Clone, Copy)]
enum Tamper {
    Subject,
    Reason,
    Time,
    Nonce,
    KeyId,
    Kind,
    Signature,
}

fn tampers() -> impl Strategy<Value = Tamper> {
    prop_oneof![
        Just(Tamper::Subject),
        Just(Tamper::Reason),
        Just(Tamper::Time),
        Just(Tamper::Nonce),
        Just(Tamper::KeyId),
        Just(Tamper::Kind),
        Just(Tamper::Signature),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn p2p_any_change_breaks_the_signature(
        name in "[a-z][a-z0-9]{0,12}",
        tld in "[a-z]{2,6}",
        tamper in tampers(),
        shift in 1i64..100_000,
    ) {
        let a = key();
        let b = key();
        let both = keys(&[("apk_a", &a), ("apk_b", &b)]);
        let r = domain_report(&a, "apk_a", &format!("{name}.{tld}"), NOW);
        prop_assert!(r.verifies(&both));
        let mut t = r.clone();
        match tamper {
            Tamper::Subject => t.subject = format!("x{}", r.subject),
            Tamper::Reason => t.reason = Reason::Malware,
            Tamper::Time => t.signed_at += shift,
            Tamper::Nonce => t.nonce = fresh_nonce().expect("nonce"),
            // Another real key's id: the signature must bind the key, not only verify.
            Tamper::KeyId => t.key_id = "apk_b".into(),
            Tamper::Kind => t.kind = Kind::Handle,
            Tamper::Signature => t.signature = b.sign(r.message().as_bytes()).expect("signed"),
        }
        prop_assert_ne!(&t, &r);
        prop_assert!(!t.verifies(&both), "{:?} still verifies", tamper);
    }

    #[test]
    fn p2p_parse_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..1500)) {
        let _ = Report::parse(&bytes);
    }

    #[test]
    fn p2p_parse_never_panics_on_near_records(
        subject in ".{0,80}",
        nonce in "[A-Za-z0-9_-]{0,30}",
        key_id in ".{0,70}",
        at in any::<i64>(),
    ) {
        let v = serde_json::json!({
            "v": 1, "kind": "handle", "subject": subject, "reason": "phishing",
            "signed_at": at, "nonce": nonce, "key_id": key_id, "signature": "AA",
        });
        if let Ok(r) = Report::parse(&serde_json::to_vec(&v).expect("json")) {
            prop_assert!(r.check().is_ok());
        }
    }

    #[test]
    fn p2p_normalise_is_idempotent(raw in ".{0,80}") {
        for kind in [Kind::Domain, Kind::Handle] {
            if let Ok(n) = normalise(kind, &raw) {
                prop_assert_eq!(normalise(kind, &n), Ok(n.clone()));
            }
        }
    }
}

// The guard: flood and smear.

#[test]
fn p2p_one_key_signs_ten_an_hour() {
    let k = key();
    let mut g = Guard::new(Limits::default(), keys(&[("apk_a", &k)]));
    for i in 0..10 {
        let r = domain_report(&k, "apk_a", &format!("s{i}.example"), NOW + i);
        g.accept(&r, "rp_1", NOW + i, Freshness::Live)
            .expect("accepted");
    }
    let r = domain_report(&k, "apk_a", "s10.example", NOW + 10);
    assert_eq!(
        g.accept(&r, "rp_1", NOW + 10, Freshness::Live),
        Err(Refusal::KeyRate)
    );
    // An hour on, the key may sign again.
    let later = NOW + HOUR + 1;
    let r = domain_report(&k, "apk_a", "s11.example", later);
    g.accept(&r, "rp_1", later, Freshness::Live)
        .expect("accepted an hour on");
}

#[test]
fn p2p_a_reporter_with_many_keys_is_one_reporter_for_the_day() {
    let ks: Vec<RunKey> = (0..4).map(|_| key()).collect();
    let ids = ["apk_a", "apk_b", "apk_c", "apk_d"];
    let pairs: Vec<(&str, &RunKey)> = ids.iter().copied().zip(ks.iter()).collect();
    let mut g = Guard::new(Limits::default(), keys(&pairs));
    for (n, (id, k)) in pairs.iter().take(3).enumerate() {
        for i in 0..10 {
            let at = NOW + i64::try_from(n * 10 + i).expect("small");
            let r = domain_report(k, id, &format!("k{n}s{i}.example"), at);
            g.accept(&r, "rp_1", at, Freshness::Live).expect("accepted");
        }
    }
    let r = domain_report(&ks[3], "apk_d", "fresh.example", NOW + 40);
    assert_eq!(
        g.accept(&r, "rp_1", NOW + 40, Freshness::Live),
        Err(Refusal::ReporterRate)
    );
    // Another person is not held by the first one's limit.
    g.accept(&r, "rp_2", NOW + 40, Freshness::Live)
        .expect("another reporter");
}

#[test]
fn p2p_replays_are_refused() {
    let k = key();
    let mut g = Guard::new(Limits::default(), keys(&[("apk_a", &k)]));
    let r = domain_report(&k, "apk_a", "shop.example", NOW);
    g.accept(&r, "rp_1", NOW, Freshness::Live)
        .expect("accepted");
    assert_eq!(
        g.accept(&r, "rp_1", NOW + 1, Freshness::Live),
        Err(Refusal::Replay)
    );
    assert_eq!(
        g.accept(&r, "rp_2", NOW + 1, Freshness::Stored),
        Err(Refusal::Replay)
    );
    // The same nonce over another subject: the nonce is spent.
    let again = with_nonce(&k, "apk_a", "other.example", NOW, &r.nonce);
    assert!(again.verifies(&keys(&[("apk_a", &k)])));
    assert_eq!(
        g.accept(&again, "rp_1", NOW + 1, Freshness::Live),
        Err(Refusal::Replay)
    );
    assert_eq!(g.len(), 1);
}

#[test]
fn p2p_time_is_checked() {
    let k = key();
    let ks = keys(&[("apk_a", &k)]);
    let mut g = Guard::new(Limits::default(), ks);
    let r = domain_report(&k, "apk_a", "shop.example", NOW);
    // Live: within ten minutes either way.
    assert_eq!(
        g.accept(&r, "rp_1", NOW + 601, Freshness::Live),
        Err(Refusal::Stale)
    );
    assert_eq!(
        g.accept(&r, "rp_1", NOW - 601, Freshness::Live),
        Err(Refusal::Future)
    );
    assert_eq!(
        g.accept(&r, "rp_1", NOW - 601, Freshness::Stored),
        Err(Refusal::Future)
    );
    // Stored: up to thirty days old.
    assert_eq!(
        g.accept(&r, "rp_1", NOW + 30 * DAY + 1, Freshness::Stored),
        Err(Refusal::Stale)
    );
    g.accept(&r, "rp_1", NOW + 2 * DAY, Freshness::Stored)
        .expect("catching up");
}

#[test]
fn p2p_one_person_cannot_list_a_subject_alone() {
    let one = key();
    let two = key();
    let three = key();
    let ks = keys(&[("apk_a", &one), ("apk_b", &two), ("apk_c", &three)]);
    let mut g = Guard::new(Limits::default(), ks);
    // One reporter, three keys.
    for (id, k) in [("apk_a", &one), ("apk_b", &two), ("apk_c", &three)] {
        let r = domain_report(k, id, "victim.example", NOW);
        g.accept(&r, "rp_1", NOW, Freshness::Live)
            .expect("accepted");
    }
    assert_eq!(g.listed(Kind::Domain, "victim.example", NOW), None);
    // Three distinct reporters list it.
    let r = domain_report(&two, "apk_b", "victim.example", NOW + 1);
    g.accept(&r, "rp_2", NOW + 1, Freshness::Live)
        .expect("accepted");
    assert_eq!(g.listed(Kind::Domain, "victim.example", NOW + 1), None);
    let r = domain_report(&three, "apk_c", "victim.example", NOW + 2);
    g.accept(&r, "rp_3", NOW + 2, Freshness::Live)
        .expect("accepted");
    let listed = g
        .listed(Kind::Domain, "victim.example", NOW + 2)
        .expect("listed");
    assert_eq!(listed.reporters, 3);
    assert!(listed.reasons.contains(&Reason::FakeShop));
    // A week on, the listing lapses; a handle of the same text is another subject.
    assert_eq!(
        g.listed(Kind::Domain, "victim.example", NOW + 8 * DAY),
        None
    );
    assert_eq!(g.listed(Kind::Handle, "victim.example", NOW + 2), None);
}

#[test]
fn p2p_retraction_is_the_signers_and_refunds_nothing() {
    let ks: Vec<RunKey> = (0..3).map(|_| key()).collect();
    let ids = ["apk_a", "apk_b", "apk_c"];
    let pairs: Vec<(&str, &RunKey)> = ids.iter().copied().zip(ks.iter()).collect();
    let mut g = Guard::new(Limits::default(), keys(&pairs));
    let mut first = Vec::new();
    for (n, (id, k)) in pairs.iter().enumerate() {
        let r = domain_report(k, id, "shop.example", NOW);
        first.push(
            g.accept(&r, &format!("rp_{n}"), NOW, Freshness::Live)
                .expect("accepted"),
        );
    }
    assert!(g.listed(Kind::Domain, "shop.example", NOW).is_some());
    assert_eq!(g.retract(&first[0], "apk_b"), Err(Refusal::NotOwner));
    assert_eq!(g.retract("no-such-id", "apk_a"), Err(Refusal::Unknown));
    g.retract(&first[0], "apk_a").expect("retracted");
    assert_eq!(g.listed(Kind::Domain, "shop.example", NOW), None);
    assert_eq!(g.retract(&first[0], "apk_a"), Err(Refusal::Unknown));
    // Sign-retract cycles do not dodge the hourly limit: 9 more fill it.
    let (id, k) = pairs[0];
    for i in 0..9 {
        let r = domain_report(k, id, &format!("c{i}.example"), NOW + 1 + i);
        let rid = g
            .accept(&r, "rp_0", NOW + 1 + i, Freshness::Live)
            .expect("accepted");
        g.retract(&rid, id).expect("retracted");
    }
    let r = domain_report(k, id, "c9.example", NOW + 20);
    assert_eq!(
        g.accept(&r, "rp_0", NOW + 20, Freshness::Live),
        Err(Refusal::KeyRate)
    );
}

#[test]
fn p2p_fake_reports_spend_nothing() {
    let victim = key();
    let forger = key();
    let mut g = Guard::new(Limits::default(), keys(&[("apk_v", &victim)]));
    // A forger signs under the victim's key id, many times, and reuses a nonce the victim
    // will use next.
    let nonce = fresh_nonce().expect("nonce");
    for i in 0..50 {
        let fake = with_nonce(&forger, "apk_v", &format!("f{i}.example"), NOW, &nonce);
        assert_eq!(
            g.accept(&fake, "rp_v", NOW, Freshness::Live),
            Err(Refusal::Signature)
        );
    }
    assert_eq!(g.len(), 0);
    // The victim's quota, nonce and reporter count are untouched.
    let real = with_nonce(&victim, "apk_v", "real.example", NOW, &nonce);
    g.accept(&real, "rp_v", NOW, Freshness::Live)
        .expect("the nonce is still free");
    for i in 0..9 {
        let r = domain_report(&victim, "apk_v", &format!("v{i}.example"), NOW + 1 + i);
        g.accept(&r, "rp_v", NOW + 1 + i, Freshness::Live)
            .expect("quota untouched");
    }
}

#[test]
fn p2p_malformed_reports_and_a_full_store_are_refused() {
    let k = key();
    let limits = Limits {
        max_reports: 2,
        ..Limits::default()
    };
    let mut g = Guard::new(limits, keys(&[("apk_a", &k)]));
    let r = domain_report(&k, "apk_a", "a.example", NOW);
    assert_eq!(g.accept(&r, "", NOW, Freshness::Live), Err(Refusal::Shape));
    let mut upper = r.clone();
    upper.subject = "A.example".into();
    assert_eq!(
        g.accept(&upper, "rp_1", NOW, Freshness::Live),
        Err(Refusal::Shape)
    );
    g.accept(&r, "rp_1", NOW, Freshness::Live)
        .expect("accepted");
    let r = domain_report(&k, "apk_a", "b.example", NOW);
    g.accept(&r, "rp_1", NOW, Freshness::Live)
        .expect("accepted");
    let r = domain_report(&k, "apk_a", "c.example", NOW);
    assert_eq!(
        g.accept(&r, "rp_1", NOW, Freshness::Live),
        Err(Refusal::Full)
    );
    // Thirty days on, the old ones expire and make room.
    let later = NOW + 30 * DAY + 1;
    let r = domain_report(&k, "apk_a", "c.example", later);
    g.accept(&r, "rp_1", later, Freshness::Live)
        .expect("room after expiry");
    assert_eq!(g.len(), 1);
}

#[test]
fn p2p_refusals_name_no_subject() {
    for r in [
        Refusal::Shape,
        Refusal::Stale,
        Refusal::Future,
        Refusal::Signature,
        Refusal::Replay,
        Refusal::KeyRate,
        Refusal::ReporterRate,
        Refusal::Full,
        Refusal::NotOwner,
        Refusal::Unknown,
    ] {
        assert!(r.as_str().len() > 3);
        assert!(!r.as_str().contains('.'));
    }
}

// The agent.

#[test]
fn p2p_bootstrap_addresses_are_checked() {
    for (raw, want) in [
        ("https://boot.inorbit.hr", "https://boot.inorbit.hr/"),
        ("https://Boot.Example:4433/", "https://boot.example:4433/"),
        ("http://127.0.0.1:9", "http://127.0.0.1:9/"),
        ("http://localhost:9", "http://localhost:9/"),
        ("http://[::1]:9", "http://[::1]:9/"),
    ] {
        assert_eq!(
            Bootstrap::named(Some(raw)).expect(raw).as_str(),
            want,
            "{raw}"
        );
    }
    for raw in [
        "http://boot.example",
        "http://10.0.0.1:9",
        "ftp://boot.example",
        "https://user:pw@boot.example",
        "https://boot.example/path",
        "https://boot.example/?q=1",
        "https://boot.example/#f",
        "boot.example",
        "not a url",
    ] {
        let e = Bootstrap::named(Some(raw)).expect_err(raw);
        assert!(!e.contains("pw"), "{e}");
    }
    let e = Bootstrap::named(None).expect_err("none");
    assert!(e.contains(agent::BOOTSTRAP_ENV), "{e}");
    assert!(Bootstrap::named(Some("  ")).is_err());
}

fn quick() -> Timing {
    Timing {
        tick: Duration::from_millis(5),
        probe_timeout: Duration::from_millis(500),
        reachable_every: Duration::from_millis(30),
        first_retry: Duration::from_millis(5),
        max_backoff: Duration::from_millis(20),
    }
}

async fn until(dir: &std::path::Path, what: impl Fn(&State) -> bool) -> State {
    for _ in 0..1000 {
        if let Some(s) = agent::read_state(dir).filter(|s| what(s)) {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("the agent never got there: {:?}", agent::read_state(dir));
}

fn start(dir: PathBuf, b: Bootstrap) -> impl FnOnce() -> Result<u32, String> {
    move || {
        tokio::spawn(async move { agent::run(&dir, &b, quick()).await });
        Ok(4242)
    }
}

#[tokio::test]
async fn p2p_register_reaches_the_node_and_unregister_stops_it() {
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = agent::dir(tmp.path());
    let node = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = node.local_addr().expect("addr").port();
    let b = Bootstrap::parse(&format!("http://127.0.0.1:{port}")).expect("address");

    let done =
        agent::register(&dir, &b, agent_now(), start(dir.clone(), b.clone())).expect("registered");
    assert_eq!(done, Registered::Started(4242));
    let s = until(&dir, |s| s.phase == Phase::Reachable).await;
    assert!(s.last_contact_at.is_some());
    assert_eq!(s.bootstrap, b.as_str());
    assert_eq!(s.pid, std::process::id());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(dir.join("state.json"))
            .expect("state")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // A second register while it runs starts nothing.
    let again =
        agent::register(&dir, &b, agent_now(), || panic!("started twice")).expect("registered");
    assert!(matches!(again, Registered::AlreadyRunning(_)));

    assert_eq!(
        agent::unregister(&dir, Duration::from_secs(5), Duration::from_millis(5)).await,
        Ok(true)
    );
    assert!(!dir.exists());
    assert_eq!(
        agent::unregister(&dir, Duration::from_secs(5), Duration::from_millis(5)).await,
        Ok(false)
    );
}

#[tokio::test]
async fn p2p_an_unreachable_node_is_retried_with_backoff() {
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = agent::dir(tmp.path());
    // A port nobody listens on.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        l.local_addr().expect("addr").port()
    };
    let b = Bootstrap::parse(&format!("http://127.0.0.1:{port}")).expect("address");
    agent::register(&dir, &b, agent_now(), start(dir.clone(), b.clone())).expect("registered");
    let s = until(&dir, |s| s.phase == Phase::Unreachable && s.attempts >= 4).await;
    assert_eq!(s.last_contact_at, None);
    assert!(s.running(agent_now()));
    agent::unregister(&dir, Duration::from_secs(5), Duration::from_millis(5))
        .await
        .expect("stopped");
}

#[test]
fn p2p_a_silent_agent_is_not_running() {
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = agent::dir(tmp.path());
    let b = Bootstrap::parse("https://boot.example").expect("address");
    let now = agent_now();
    let dead = State {
        bootstrap: b.as_str().into(),
        pid: 1,
        started_at: now - 1000,
        phase: Phase::Reachable,
        last_contact_at: Some(now - 1000),
        attempts: 3,
        updated_at: now - agent::FRESH - 1,
    };
    assert!(!dead.running(now));
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(
        dir.join("state.json"),
        serde_json::to_vec(&dead).expect("json"),
    )
    .expect("write");
    // register starts a new agent over a dead one, and clears an old stop request.
    std::fs::write(dir.join("stop"), b"stop\n").expect("write");
    let done = agent::register(&dir, &b, now, || Ok(7)).expect("registered");
    assert_eq!(done, Registered::Started(7));
    assert!(!dir.join("stop").exists());
    assert_eq!(
        agent::read_state(&dir).map(|s| s.phase),
        Some(Phase::Starting)
    );
}

fn agent_now() -> i64 {
    crate::runner::approvals::unix_now()
}
