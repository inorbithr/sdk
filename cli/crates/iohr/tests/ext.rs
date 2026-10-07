//! Extensions end to end against an in-process OCI registry: install verifies digest
//! and signature, refuses tampering and strangers, pins a lock, syncs from it, and runs
//! the program with a token channel that hands out only declared scopes.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use iohr::ext::layer::tar_gz;
use iohr::ext::oci::Digest;
use sigstore_verify::crypto::KeyPair;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const PREFIX: &str = "iohr-ext";

fn platform() -> serde_json::Value {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        o => o,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        a => a,
    };
    serde_json::json!({"os": os, "architecture": arch})
}

fn token(scopes: &[&str]) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let enc = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.TOKENSIGNATUREMARKER",
        enc(serde_json::json!({"alg": "RS256"})),
        enc(serde_json::json!({
            "sub": "ak_tok1", "aud": ["iohr-api"], "exp": 4_102_444_800_i64,
            "scp": scopes, "org": "acc_test1"
        }))
    )
}

/// One extension version as a registry holds it.
struct Artifact {
    index: Vec<u8>,
    manifest: Vec<u8>,
    config: Vec<u8>,
    layer: Vec<u8>,
}

impl Artifact {
    fn new(name: &str, version: &str, scopes: &[&str], program: &[u8]) -> Self {
        Self::privileged(name, version, scopes, &[], program)
    }

    /// An artifact whose manifest declares `privileges` (left out when empty, as an
    /// older manifest has none).
    fn privileged(
        name: &str,
        version: &str,
        scopes: &[&str],
        privileges: &[&str],
        program: &[u8],
    ) -> Self {
        let mut config = serde_json::json!({
            "name": name, "version": version, "entrypoint": "bin/prog",
            "scopes": scopes, "description": "a test extension"
        });
        if !privileges.is_empty() {
            config["privileges"] = serde_json::json!(privileges);
        }
        let config = config.to_string().into_bytes();
        let layer = tar_gz(&[("README", b'0', b"hi"), ("bin/prog", b'0', program)]);
        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "config": {"mediaType": "application/vnd.inorbit.iohr.extension.config.v1+json",
                       "digest": Digest::of(&config).as_str(), "size": config.len()},
            "layers": [{"mediaType": "application/vnd.inorbit.iohr.extension.layer.v1.tar+gzip",
                        "digest": Digest::of(&layer).as_str(), "size": layer.len()}]
        })
        .to_string()
        .into_bytes();
        let index = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "manifests": [
                {"mediaType": "application/vnd.oci.image.manifest.v1+json",
                 "digest": Digest::of(&manifest).as_str(), "size": manifest.len(),
                 "platform": platform()},
                {"mediaType": "application/vnd.oci.image.manifest.v1+json",
                 "digest": format!("sha256:{}", "0".repeat(64)), "size": 1,
                 "platform": {"os": "plan9", "architecture": "mips"}}
            ]
        })
        .to_string()
        .into_bytes();
        Self {
            index,
            manifest,
            config,
            layer,
        }
    }

    fn index_digest(&self) -> Digest {
        Digest::of(&self.index)
    }
}

/// A Sigstore bundle by `key` over a statement naming `digest`.
fn bundle(key: &KeyPair, digest: &Digest) -> Vec<u8> {
    let statement = serde_json::json!({
        "_type": "https://in-toto.io/Statement/v1",
        "subject": [{"name": "", "digest": {"sha256": digest.hex()}}],
        "predicateType": "https://sigstore.dev/cosign/sign/v1",
        "predicate": {}
    })
    .to_string();
    let pae = sigstore_verify::types::pae("application/vnd.in-toto+json", statement.as_bytes());
    let sig = key.sign(&pae).unwrap();
    serde_json::json!({
        "mediaType": "application/vnd.dev.sigstore.bundle.v0.3+json",
        "verificationMaterial": {"publicKey": {"hint": ""}},
        "dsseEnvelope": {
            "payload": STANDARD.encode(statement),
            "payloadType": "application/vnd.in-toto+json",
            "signatures": [{"sig": STANDARD.encode(sig.as_bytes())}]
        }
    })
    .to_string()
    .into_bytes()
}

fn blob(bytes: &[u8]) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_bytes(bytes.to_vec())
}

fn doc(bytes: &[u8], media_type: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(bytes.to_vec(), media_type)
}

/// Mounts `a` as `name:version`, signed by `key` (or unsigned), on `server`.
async fn publish(
    server: &MockServer,
    name: &str,
    version: &str,
    a: &Artifact,
    key: Option<&KeyPair>,
) {
    let repo = format!("/v2/{PREFIX}/{name}");
    let index_d = a.index_digest();
    for reference in [version.to_owned(), index_d.to_string()] {
        Mock::given(method("GET"))
            .and(path(format!("{repo}/manifests/{reference}")))
            .respond_with(doc(&a.index, "application/vnd.oci.image.index.v1+json"))
            .mount(server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path(format!(
            "{repo}/manifests/{}",
            Digest::of(&a.manifest)
        )))
        .respond_with(doc(
            &a.manifest,
            "application/vnd.oci.image.manifest.v1+json",
        ))
        .mount(server)
        .await;
    for b in [&a.config, &a.layer] {
        Mock::given(method("GET"))
            .and(path(format!("{repo}/blobs/{}", Digest::of(b))))
            .respond_with(blob(b))
            .mount(server)
            .await;
    }
    let referrers = if let Some(key) = key {
        let bundle = bundle(key, &index_d);
        let bundle_d = Digest::of(&bundle);
        let empty = b"{}".to_vec();
        let referrer = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "artifactType": "application/vnd.dev.sigstore.bundle.v0.3+json",
            "config": {"mediaType": "application/vnd.oci.empty.v1+json",
                       "digest": Digest::of(&empty).as_str(), "size": 2},
            "layers": [{"mediaType": "application/vnd.dev.sigstore.bundle.v0.3+json",
                        "digest": bundle_d.as_str(), "size": bundle.len()}],
            "subject": {"mediaType": "application/vnd.oci.image.index.v1+json",
                        "digest": index_d.as_str(), "size": a.index.len()}
        })
        .to_string()
        .into_bytes();
        Mock::given(method("GET"))
            .and(path(format!("{repo}/manifests/{}", Digest::of(&referrer))))
            .respond_with(doc(&referrer, "application/vnd.oci.image.manifest.v1+json"))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("{repo}/blobs/{bundle_d}")))
            .respond_with(blob(&bundle))
            .mount(server)
            .await;
        serde_json::json!([{
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "digest": Digest::of(&referrer).as_str(), "size": referrer.len(),
            "artifactType": "application/vnd.dev.sigstore.bundle.v0.3+json"
        }])
    } else {
        serde_json::json!([])
    };
    // The index's referrers through the API; the platform manifest has none (404,
    // then the tag schema, also 404).
    Mock::given(method("GET"))
        .and(path(format!("{repo}/referrers/{index_d}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "manifests": referrers
        })))
        .mount(server)
        .await;
}

async fn tags(server: &MockServer, name: &str, tags: &[&str]) {
    Mock::given(method("GET"))
        .and(path(format!("/v2/{PREFIX}/{name}/tags/list")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": format!("{PREFIX}/{name}"), "tags": tags
        })))
        .mount(server)
        .await;
}

struct Box_ {
    server: MockServer,
    dir: tempfile::TempDir,
}

impl Box_ {
    async fn new() -> Self {
        Self {
            server: MockServer::start().await,
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_iohr"));
        c.args(args)
            .env_clear()
            .env("IOHR_BASE_URL", "http://127.0.0.1:9")
            .env("IOHR_CONFIG_DIR", self.dir.path().join("config"))
            .env("IOHR_DATA_DIR", self.dir.path().join("data"))
            .env(
                "IOHR_EXT_REGISTRY",
                format!("{}/{PREFIX}", self.server.uri()),
            )
            .current_dir(self.dir.path());
        for k in ["PATH", "SYSTEMROOT", "TEMP", "TMP", "TMPDIR", "HOME"] {
            if let Some(v) = std::env::var_os(k) {
                c.env(k, v);
            }
        }
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd(args).stdin(Stdio::null()).output().unwrap()
    }

    fn trust(&self, key: &KeyPair) {
        let pem = self.dir.path().join("trusted.pem");
        std::fs::write(&pem, key.public_key_der().unwrap().to_pem()).unwrap();
        let o = self.run(&["config", "set", "ext.trusted_keys", pem.to_str().unwrap()]);
        assert_eq!(code(&o), 0, "{}", text(&o));
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("data/extensions")
    }
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}

fn nothing_installed(b: &Box_) {
    let o = b.run(&["ext", "list", "--json"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "[]");
}

#[tokio::test(flavor = "multi_thread")]
async fn install_verifies_pins_and_lists() {
    let b = Box_::new().await;
    let key = KeyPair::generate_ecdsa_p256().unwrap();
    b.trust(&key);
    let a1 = Artifact::new("agent", "0.1.0", &["agents:write"], b"v1");
    let a2 = Artifact::new("agent", "0.2.0", &["agents:write", "domains:read"], b"v2");
    publish(&b.server, "agent", "0.1.0", &a1, Some(&key)).await;
    publish(&b.server, "agent", "0.2.0", &a2, Some(&key)).await;
    tags(
        &b.server,
        "agent",
        &["0.1.0", "0.2.0", "0.3.0-rc.1", "latest"],
    )
    .await;

    let o = b.run(&["ext", "install", "agent@0.1.0", "--lock", "team.lock"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(
        text(&o).contains("may ask for agents:write"),
        "{}",
        text(&o)
    );
    let lock = std::fs::read_to_string(b.dir.path().join("team.lock")).unwrap();
    assert!(lock.contains(a1.index_digest().as_str()), "{lock}");
    assert!(lock.contains("signer = \"key:sha256:"), "{lock}");

    let o = b.run(&["ext", "list"]);
    assert!(
        text(&o).contains("agent") && text(&o).contains("0.1.0"),
        "{}",
        text(&o)
    );
    let o = b.run(&["ext", "verify"]);
    assert_eq!(code(&o), 0, "{}", text(&o));

    // Upgrade takes the newest release, not the release candidate, and names new scopes.
    let o = b.run(&["ext", "upgrade"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(
        text(&o).contains("0.2.0") && text(&o).contains("new scopes  domains:read"),
        "{}",
        text(&o)
    );
    let o = b.run(&["ext", "upgrade", "agent"]);
    assert!(text(&o).contains("is the newest"), "{}", text(&o));
    // One version on disk at a time.
    assert_eq!(
        std::fs::read_dir(b.data().join("agent")).unwrap().count(),
        1
    );

    let o = b.run(&["ext", "remove", "agent"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    nothing_installed(&b);
    assert!(!b.data().join("agent").exists());

    // The team's lock brings back exactly 0.1.0.
    let o = b.run(&["ext", "sync", "--lock", "team.lock"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let o = b.run(&["ext", "list", "--json"]);
    assert!(
        String::from_utf8_lossy(&o.stdout).contains("\"0.1.0\""),
        "{}",
        text(&o)
    );
}

fn stdout_json(o: &Output) -> serde_json::Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}", text(o)))
}

/// Publishes `capture` 0.1.0 (`CAP_BPF`, `CAP_PERFMON`) and 0.2.0 (`CAP_NET_ADMIN`
/// added); returns 0.2.0.
async fn publish_capture(b: &Box_, key: &KeyPair) -> Artifact {
    let a1 = Artifact::privileged(
        "capture",
        "0.1.0",
        &["agents:read"],
        &["CAP_BPF", "CAP_PERFMON"],
        b"v1",
    );
    let a2 = Artifact::privileged(
        "capture",
        "0.2.0",
        &["agents:read"],
        &["CAP_BPF", "CAP_PERFMON", "CAP_NET_ADMIN"],
        b"v2",
    );
    publish(&b.server, "capture", "0.1.0", &a1, Some(key)).await;
    publish(&b.server, "capture", "0.2.0", &a2, Some(key)).await;
    tags(&b.server, "capture", &["0.1.0", "0.2.0"]).await;
    a2
}

#[tokio::test(flavor = "multi_thread")]
async fn privileges_are_confirmed_shown_and_pinned() {
    let b = Box_::new().await;
    let key = KeyPair::generate_ecdsa_p256().unwrap();
    b.trust(&key);
    let a2 = publish_capture(&b, &key).await;

    // No terminal and no --yes: the privileges are shown in plain words, nothing installed.
    let o = b.run(&["ext", "install", "capture@0.1.0", "--lock", "team.lock"]);
    assert_eq!(code(&o), 2, "{}", text(&o));
    for want in [
        "declares privileges",
        "CAP_BPF      load eBPF programs into the kernel",
        "iohr runs as you and grants none of them",
        "confirm them at a terminal, or pass --yes; nothing was installed",
    ] {
        assert!(text(&o).contains(want), "{want}: {}", text(&o));
    }
    nothing_installed(&b);
    assert!(!b.dir.path().join("team.lock").exists());

    // --yes confirms; the JSON result and both locks carry the privileges.
    let o = b.run(&[
        "ext",
        "install",
        "capture@0.1.0",
        "--lock",
        "team.lock",
        "--yes",
        "--json",
    ]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert_eq!(
        stdout_json(&o)["privileges"],
        serde_json::json!(["CAP_BPF", "CAP_PERFMON"])
    );
    for lock in [
        b.dir.path().join("team.lock"),
        b.data().join("iohr-ext.lock"),
    ] {
        let lock = std::fs::read_to_string(lock).unwrap();
        assert!(lock.contains("version = 2\n"), "{lock}");
        assert!(
            lock.contains("privileges = [\n    \"CAP_BPF\",\n    \"CAP_PERFMON\",\n]"),
            "{lock}"
        );
    }

    // list and verify show them, as a column and in JSON.
    let o = b.run(&["ext", "list"]);
    assert!(
        text(&o).contains("PRIVILEGES") && text(&o).contains("CAP_BPF CAP_PERFMON"),
        "{}",
        text(&o)
    );
    let o = b.run(&["ext", "list", "--json"]);
    assert_eq!(
        stdout_json(&o)[0]["privileges"],
        serde_json::json!(["CAP_BPF", "CAP_PERFMON"])
    );
    let o = b.run(&["ext", "verify", "--json"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert_eq!(
        stdout_json(&o)[0]["privileges"],
        serde_json::json!(["CAP_BPF", "CAP_PERFMON"])
    );

    // A release with a new privilege asks again: refused without a terminal or --yes.
    let o = b.run(&["ext", "upgrade", "capture"]);
    assert_eq!(code(&o), 2, "{}", text(&o));
    assert!(
        text(&o).contains("declares new privileges (CAP_NET_ADMIN)"),
        "{}",
        text(&o)
    );
    let o = b.run(&["ext", "list", "--json"]);
    assert_eq!(stdout_json(&o)[0]["version"], "0.1.0");
    let o = b.run(&["ext", "upgrade", "capture", "--yes"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(
        text(&o).contains("new privileges CAP_NET_ADMIN"),
        "{}",
        text(&o)
    );

    // sync refuses a pinned extension whose privileges grew beyond what the lock records.
    assert_eq!(code(&b.run(&["ext", "remove", "capture"])), 0);
    let grown = format!(
        "version = 2\n\n[[extension]]\nname = \"capture\"\nversion = \"0.2.0\"\ndigest = \"{}\"\n\
         signer = \"{}\"\nprivileges = [\"CAP_BPF\", \"CAP_PERFMON\"]\n",
        a2.index_digest(),
        signer(&b)
    );
    std::fs::write(b.dir.path().join("grown.lock"), grown).unwrap();
    let o = b.run(&["ext", "sync", "--lock", "grown.lock"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(
        text(&o).contains("does not record (CAP_NET_ADMIN): not installed"),
        "{}",
        text(&o)
    );
    nothing_installed(&b);

    // The team's lock, which records what was confirmed, installs without asking.
    let o = b.run(&["ext", "sync", "--lock", "team.lock"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(
        text(&o).contains("holds CAP_BPF, CAP_PERFMON (confirmed in team.lock)"),
        "{}",
        text(&o)
    );
}

/// The signer the team lock recorded, read back from it.
fn signer(b: &Box_) -> String {
    let lock = std::fs::read_to_string(b.dir.path().join("team.lock")).unwrap();
    lock.lines()
        .find_map(|l| l.strip_prefix("signer = \""))
        .and_then(|l| l.strip_suffix('"'))
        .unwrap()
        .to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn privileges_never_confirmed_do_not_run() {
    let b = Box_::new().await;
    let key = KeyPair::generate_ecdsa_p256().unwrap();
    b.trust(&key);
    let a = Artifact::privileged(
        "capture",
        "1.0.0",
        &[],
        &["CAP_BPF"],
        b"#!/bin/sh\nexit 0\n",
    );
    publish(&b.server, "capture", "1.0.0", &a, Some(&key)).await;
    let o = b.run(&["ext", "install", "capture@1.0.0", "--yes"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    // The machine's lock no longer records the confirmation (as if edited by hand).
    let path = b.data().join("iohr-ext.lock");
    let lock = std::fs::read_to_string(&path).unwrap();
    let edited = lock
        .replace("version = 2", "version = 1")
        .replace("privileges = [\"CAP_BPF\"]\n", "");
    assert_ne!(lock, edited);
    std::fs::write(&path, edited).unwrap();
    let o = b.run(&["capture"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(
        text(&o).contains("declares privileges that were never confirmed (CAP_BPF)"),
        "{}",
        text(&o)
    );
    let o = b.run(&["ext", "verify", "capture"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("never confirmed"), "{}", text(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unsigned_or_stranger_signed_artifact_is_refused() {
    let b = Box_::new().await;
    let trusted = KeyPair::generate_ecdsa_p256().unwrap();
    let stranger = KeyPair::generate_ecdsa_p256().unwrap();
    b.trust(&trusted);
    let a = Artifact::new("agent", "1.0.0", &[], b"prog");
    publish(&b.server, "agent", "1.0.0", &a, Some(&stranger)).await;
    let c = Artifact::new("cleaner", "1.0.0", &[], b"prog");
    publish(&b.server, "cleaner", "1.0.0", &c, None).await;

    let o = b.run(&["ext", "install", "agent@1.0.0"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("no trusted signature"), "{}", text(&o));
    let o = b.run(&["ext", "install", "cleaner@1.0.0"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("no Sigstore bundle"), "{}", text(&o));
    nothing_installed(&b);
    // Nothing half-written either.
    let leftovers: Vec<_> = std::fs::read_dir(b.data())
        .map(|d| d.filter_map(Result::ok).map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(
        leftovers
            .iter()
            .all(|n| n.to_string_lossy() == "iohr-ext.lock"),
        "{leftovers:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tampered_blob_is_refused() {
    let b = Box_::new().await;
    let key = KeyPair::generate_ecdsa_p256().unwrap();
    b.trust(&key);
    let a = Artifact::new("agent", "1.0.0", &[], b"prog");
    // Mounted first, so it wins: the layer's bytes are not the ones its digest names.
    Mock::given(method("GET"))
        .and(path(format!(
            "/v2/{PREFIX}/agent/blobs/{}",
            Digest::of(&a.layer)
        )))
        .respond_with(blob(&tar_gz(&[("bin/prog", b'0', b"evil")])))
        .mount(&b.server)
        .await;
    publish(&b.server, "agent", "1.0.0", &a, Some(&key)).await;
    let o = b.run(&["ext", "install", "agent@1.0.0"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("does not match"), "{}", text(&o));
    nothing_installed(&b);
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_refuses_a_different_signer_than_the_lock_pins() {
    let b = Box_::new().await;
    let key = KeyPair::generate_ecdsa_p256().unwrap();
    b.trust(&key);
    let a = Artifact::new("agent", "1.0.0", &[], b"prog");
    publish(&b.server, "agent", "1.0.0", &a, Some(&key)).await;
    let lock = format!(
        "version = 1\n\n[[extension]]\nname = \"agent\"\nversion = \"1.0.0\"\ndigest = \"{}\"\nsigner = \"https://github.com/inorbithr/agent/.github/workflows/release.yml\"\n",
        a.index_digest()
    );
    std::fs::write(b.dir.path().join("iohr-ext.lock"), lock).unwrap();
    let o = b.run(&["ext", "sync"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("but the lock pins"), "{}", text(&o));
    nothing_installed(&b);
}

/// The catalogue's answers for one listing, mounted on the box's server (which is the
/// API host too, under `/v1`).
async fn catalogue(
    b: &Box_,
    publisher: &str,
    name: &str,
    listing_signer: &str,
    versions: &[serde_json::Value],
) {
    let base = format!("/v1/extensions/{publisher}/{name}");
    Mock::given(method("GET"))
        .and(path(base.clone()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "extension": {
                "publisher": publisher, "name": name, "kind": "program",
                "description": "a test extension", "visibility": "public",
                "source": "https://github.com/inorbithr/dataplane", "coming": false,
                "install": format!("iohr ext install {publisher}/{name}"),
                "publisher_name": "InOrbit", "first_party": true, "shared_with": [],
                "included_by": "", "signer": listing_signer,
                "created_at": "2026-10-06T00:00:00Z", "updated_at": "2026-10-06T00:00:00Z",
                "latest": versions.first(),
            }
        })))
        .mount(&b.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{base}/versions")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "versions": versions, "next_page_token": ""
        })))
        .mount(&b.server)
        .await;
}

/// A catalogue version entry for `a`.
fn listed(version: &str, a: &Artifact, signer: &str, scopes: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "version": version, "digest": a.index_digest().as_str(),
        "platforms": ["linux/amd64"], "entrypoint": "bin/prog", "scopes": scopes,
        "privileges": [], "signer": signer, "description": "a test extension",
        "published_at": "2026-10-06T12:00:00Z",
        "evidence": {"record_ids": [], "summary": ""}
    })
}

impl Box_ {
    /// A command that also calls the API (the catalogue) with a key's token.
    fn run_api(&self, args: &[&str]) -> Output {
        self.cmd(args)
            .env("IOHR_BASE_URL", self.server.uri())
            .env("IOHR_TOKEN", token(&["extensions:read"]))
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
}

fn fingerprint(key: &KeyPair) -> String {
    let pem = key.public_key_der().unwrap().to_pem();
    iohr::ext::trust::TrustedKey::from_pem(&pem)
        .unwrap()
        .fingerprint()
        .to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn publisher_slash_name_installs_what_the_catalogue_lists() {
    let b = Box_::new().await;
    let key = KeyPair::generate_ecdsa_p256().unwrap();
    b.trust(&key);
    let fp = fingerprint(&key);
    let a1 = Artifact::new("agent", "0.1.0", &["agents:write"], b"v1");
    let a2 = Artifact::new("agent", "0.2.0", &["agents:write"], b"v2");
    let rc = Artifact::new("agent", "0.3.0-rc.1", &["agents:write"], b"v3");
    publish(&b.server, "agent", "0.1.0", &a1, Some(&key)).await;
    publish(&b.server, "agent", "0.2.0", &a2, Some(&key)).await;
    publish(&b.server, "agent", "0.3.0-rc.1", &rc, Some(&key)).await;
    catalogue(
        &b,
        "inorbit",
        "agent",
        &fp,
        &[
            listed("0.3.0-rc.1", &rc, &fp, &["agents:write"]),
            listed("0.2.0", &a2, &fp, &["agents:write"]),
            listed("0.1.0", &a1, &fp, &["agents:write"]),
        ],
    )
    .await;

    // The newest release, not the release candidate, by the digest the catalogue lists.
    let o = b.run_api(&["ext", "install", "inorbit/agent", "--json"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let j = stdout_json(&o);
    assert_eq!(j["version"], "0.2.0", "{j}");
    assert_eq!(j["digest"], a2.index_digest().as_str(), "{j}");
    assert_eq!(j["signer"], fp.as_str(), "{j}");

    // `@version` is taken as asked; the short form still goes to the registry alone.
    let o = b.run_api(&[
        "ext",
        "install",
        "inorbit/agent@0.1.0",
        "--lock",
        "team.lock",
    ]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let lock = std::fs::read_to_string(b.dir.path().join("team.lock")).unwrap();
    assert!(lock.contains(a1.index_digest().as_str()), "{lock}");
    let o = b.run(&["ext", "install", "agent@0.2.0"]);
    assert_eq!(code(&o), 0, "{}", text(&o));

    // A version the catalogue does not list fails before the registry.
    let o = b.run_api(&["ext", "install", "inorbit/agent@9.9.9"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("no version 9.9.9"), "{}", text(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_catalogue_signer_that_is_not_the_artifacts_is_refused() {
    let b = Box_::new().await;
    let key = KeyPair::generate_ecdsa_p256().unwrap();
    b.trust(&key);
    let fp = fingerprint(&key);
    let a = Artifact::new("agent", "1.0.0", &[], b"prog");
    publish(&b.server, "agent", "1.0.0", &a, Some(&key)).await;
    // The artifact passes the trust root (a configured key signed it), but the catalogue
    // names InOrbit's release workflow: the two must agree.
    let wf = "https://github.com/inorbithr/dataplane/.github/workflows/release.yml@refs/tags/";
    catalogue(
        &b,
        "inorbit",
        "agent",
        wf,
        &[listed("1.0.0", &a, &format!("{wf}v1.0.0"), &[])],
    )
    .await;
    let o = b.run_api(&["ext", "install", "inorbit/agent"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(
        text(&o).contains(&format!(
            "signed by {fp}, but the catalogue names {wf}v1.0.0"
        )),
        "{}",
        text(&o)
    );
    nothing_installed(&b);

    // The version's signer agrees, but it is not the listing's identity.
    let b = Box_::new().await;
    b.trust(&key);
    publish(&b.server, "agent", "1.0.0", &a, Some(&key)).await;
    catalogue(&b, "inorbit", "agent", wf, &[listed("1.0.0", &a, &fp, &[])]).await;
    let o = b.run_api(&["ext", "install", "inorbit/agent"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(
        text(&o).contains("not the listing's identity"),
        "{}",
        text(&o)
    );
    nothing_installed(&b);

    // A stranger's signature still fails the trust root first, whatever the catalogue says.
    let b = Box_::new().await;
    b.trust(&key);
    let stranger = KeyPair::generate_ecdsa_p256().unwrap();
    publish(&b.server, "agent", "1.0.0", &a, Some(&stranger)).await;
    let sfp = fingerprint(&stranger);
    catalogue(
        &b,
        "inorbit",
        "agent",
        &sfp,
        &[listed("1.0.0", &a, &sfp, &[])],
    )
    .await;
    let o = b.run_api(&["ext", "install", "inorbit/agent"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("no trusted signature"), "{}", text(&o));
    nothing_installed(&b);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_artifact_must_be_what_the_catalogue_lists() {
    let b = Box_::new().await;
    let key = KeyPair::generate_ecdsa_p256().unwrap();
    b.trust(&key);
    let fp = fingerprint(&key);
    let a1 = Artifact::new("agent", "1.0.0", &["agents:write"], b"v1");
    let a2 = Artifact::new("agent", "2.0.0", &["agents:write"], b"v2");
    publish(&b.server, "agent", "1.0.0", &a1, Some(&key)).await;
    publish(&b.server, "agent", "2.0.0", &a2, Some(&key)).await;
    // 1.0.0 listed at 2.0.0's digest, and with fewer scopes than the artifact asks for.
    let wrong = listed("1.0.0", &a2, &fp, &["agents:write"]);
    catalogue(&b, "inorbit", "agent", &fp, &[wrong]).await;
    let o = b.run_api(&["ext", "install", "inorbit/agent"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(
        text(&o).contains("is version 2.0.0, the catalogue lists 1.0.0"),
        "{}",
        text(&o)
    );
    nothing_installed(&b);

    let b = Box_::new().await;
    b.trust(&key);
    publish(&b.server, "agent", "1.0.0", &a1, Some(&key)).await;
    catalogue(
        &b,
        "inorbit",
        "agent",
        &fp,
        &[listed("1.0.0", &a1, &fp, &[])],
    )
    .await;
    let o = b.run_api(&["ext", "install", "inorbit/agent"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(
        text(&o).contains("asks for scopes agents:write, the catalogue lists"),
        "{}",
        text(&o)
    );
    nothing_installed(&b);
}

#[tokio::test(flavor = "multi_thread")]
async fn search_and_show_read_the_catalogue() {
    let b = Box_::new().await;
    let art = Artifact::new("agent", "0.2.0", &["agents:write", "domains:read"], b"v");
    let wf = "https://github.com/inorbithr/dataplane/.github/workflows/release.yml@refs/tags/";
    let ver = listed(
        "0.2.0",
        &art,
        &format!("{wf}v0.2.0"),
        &["agents:write", "domains:read"],
    );
    catalogue(&b, "inorbit", "agent", wf, std::slice::from_ref(&ver)).await;
    Mock::given(method("GET"))
        .and(path("/v1/extensions/inorbit"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "listings": 2,
            "publisher": {"namespace": "inorbit", "account_id": "acc_1", "display_name": "InOrbit",
                          "domain": "inorbit.hr", "first_party": true, "created_at": ""}
        })))
        .mount(&b.server)
        .await;
    let ext = |name: &str, coming: bool, latest: Option<&serde_json::Value>| {
        serde_json::json!({
            "publisher": "inorbit", "name": name, "kind": "program", "description": "d",
            "visibility": "public", "coming": coming, "publisher_name": "InOrbit",
            "first_party": true, "latest": latest
        })
    };
    // Page one says there is more; --all follows it.
    Mock::given(method("GET"))
        .and(path("/v1/extensions"))
        .and(wiremock::matchers::query_param("page_token", "p2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "extensions": [ext("capture", true, None)], "next_page_token": ""
        })))
        .mount(&b.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/extensions"))
        .and(wiremock::matchers::query_param("query", "agent"))
        .and(wiremock::matchers::query_param("kind", "program"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "extensions": [ext("agent", false, Some(&ver))], "next_page_token": "p2"
        })))
        .mount(&b.server)
        .await;

    let o = b.run_api(&["ext", "search", "agent", "--kind", "program"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(
        out.starts_with("NAME")
            && out
                .contains("inorbit/agent  program  InOrbit (inorbit, first party)  0.2.0   public"),
        "{out}"
    );
    assert!(!out.contains("capture"), "{out}");
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("--all"),
        "{}",
        text(&o)
    );

    let o = b.run_api(&[
        "ext", "search", "agent", "--kind", "program", "--all", "--json",
    ]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let j = stdout_json(&o);
    let names: Vec<&str> = j
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["agent", "capture"]);

    let o = b.run_api(&["ext", "search", "--kind", "Program!"]);
    assert_eq!(code(&o), 2, "{}", text(&o));

    let o = b.run_api(&["ext", "show", "inorbit/agent"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let out = String::from_utf8_lossy(&o.stdout);
    for want in [
        "publisher    InOrbit (inorbit, first party), inorbit.hr",
        &format!("signer       {wf}v0.2.0"),
        "scopes       agents:write, domains:read",
        "privileges   none",
        "evidence     no evidence yet",
        "0.2.0    2026-10-06",
    ] {
        assert!(out.contains(want), "{want}\n{out}");
    }
    assert!(
        out.trim_end().ends_with("iohr ext install inorbit/agent"),
        "{out}"
    );
    let o = b.run_api(&["ext", "show", "inorbit/agent", "--json"]);
    let j = stdout_json(&o);
    assert_eq!(j["install"], "iohr ext install inorbit/agent");
    assert_eq!(j["publisher"]["domain"], "inorbit.hr");
    assert_eq!(j["versions"][0]["version"], "0.2.0");

    // Not in the catalogue (or private to another account): exit 1 with a hint.
    let o = b.run_api(&["ext", "show", "inorbit/nothing"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("iohr ext search"), "{}", text(&o));
    let o = b.run_api(&["ext", "show", "agent"]);
    assert_eq!(code(&o), 2, "{}", text(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_catalogue_install_never_replaces_another_signers_extension() {
    let b = Box_::new().await;
    let ours = KeyPair::generate_ecdsa_p256().unwrap();
    let theirs = KeyPair::generate_ecdsa_p256().unwrap();
    // Both keys are trusted, as with a company mirror that re-signs and a second channel.
    let pem = |k: &KeyPair| k.public_key_der().unwrap().to_pem();
    let (p1, p2) = (b.dir.path().join("1.pem"), b.dir.path().join("2.pem"));
    std::fs::write(&p1, pem(&ours)).unwrap();
    std::fs::write(&p2, pem(&theirs)).unwrap();
    let o = b.run(&[
        "config",
        "set",
        "ext.trusted_keys",
        p1.to_str().unwrap(),
        p2.to_str().unwrap(),
    ]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let a1 = Artifact::new("agent", "1.0.0", &[], b"ours");
    publish(&b.server, "agent", "1.0.0", &a1, Some(&ours)).await;
    let o = b.run(&["ext", "install", "agent@1.0.0"]);
    assert_eq!(code(&o), 0, "{}", text(&o));

    // acme's `agent`, from the same registry path, signed by the other key.
    let b2 = Box_::new().await;
    let a2 = Artifact::new("agent", "2.0.0", &[], b"theirs");
    publish(&b2.server, "agent", "2.0.0", &a2, Some(&theirs)).await;
    let fp = fingerprint(&theirs);
    catalogue(&b2, "acme", "agent", &fp, &[listed("2.0.0", &a2, &fp, &[])]).await;
    let o = b
        .cmd(&["ext", "install", "acme/agent"])
        .env("IOHR_EXT_REGISTRY", format!("{}/{PREFIX}", b2.server.uri()))
        .env("IOHR_BASE_URL", b2.server.uri())
        .env("IOHR_TOKEN", token(&["extensions:read"]))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(
        text(&o).contains("`iohr ext remove agent` first"),
        "{}",
        text(&o)
    );
    let o = b.run(&["ext", "list", "--json"]);
    assert!(
        String::from_utf8_lossy(&o.stdout).contains("\"1.0.0\""),
        "{}",
        text(&o)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn another_publishers_extension_needs_its_registry() {
    let b = Box_::new().await;
    // Not in InOrbit's registry: refused before any call, until ext.registry is set.
    let o = b
        .cmd(&["ext", "install", "acme/checks"])
        .env_remove("IOHR_EXT_REGISTRY")
        .env("IOHR_BASE_URL", b.server.uri())
        .env("IOHR_TOKEN", token(&["extensions:read"]))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(code(&o), 2, "{}", text(&o));
    assert!(text(&o).contains("set ext.registry"), "{}", text(&o));
    assert!(b.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_private_registry_challenge_is_answered_with_the_configured_credential() {
    let b = Box_::new().await;
    let key = KeyPair::generate_ecdsa_p256().unwrap();
    b.trust(&key);
    let a = Artifact::new("agent", "1.0.0", &[], b"prog");
    let realm = format!("{}/token", b.server.uri());
    let challenge = format!(r#"Bearer realm="{realm}",service="test""#);
    // Without the pull token every registry call is challenged.
    Mock::given(method("GET"))
        .and(path("/token"))
        .and(header("authorization", "Basic dXNlcjpwYXNz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"token": "PULLTOKEN"})),
        )
        .mount(&b.server)
        .await;
    Mock::given(method("GET"))
        .and(|r: &Request| {
            r.url.path().starts_with("/v2/")
                && r.headers.get("authorization").and_then(|v| v.to_str().ok())
                    != Some("Bearer PULLTOKEN")
        })
        .respond_with(
            ResponseTemplate::new(401).insert_header("www-authenticate", challenge.as_str()),
        )
        .mount(&b.server)
        .await;
    publish(&b.server, "agent", "1.0.0", &a, Some(&key)).await;

    let o = b.run(&["ext", "install", "agent@1.0.0"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("IOHR_EXT_REGISTRY_AUTH"), "{}", text(&o));

    let o = b
        .cmd(&["ext", "install", "agent@1.0.0", "--verbose"])
        .env("IOHR_EXT_REGISTRY_AUTH", "user:pass")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(code(&o), 0, "{}", text(&o));
    let all = text(&o);
    assert!(
        !all.contains("PULLTOKEN") && !all.contains("dXNlcjpwYXNz") && !all.contains("user:pass"),
        "{all}"
    );
}

#[cfg(unix)]
mod run {
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::Path;
    use std::time::{Duration, Instant};

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;

    /// A program that records its environment and arguments, waits for `release`, then
    /// exits 7.
    fn program(dir: &Path) -> Vec<u8> {
        format!(
            "#!/bin/sh\nenv > '{d}/env.txt'\necho \"$@\" > '{d}/args.txt'\n\
             while [ ! -e '{d}/release' ]; do sleep 0.05; done\nexit 7\n",
            d = dir.display()
        )
        .into_bytes()
    }

    async fn ask(socket: &str, body: &str) -> String {
        let mut s = tokio::net::UnixStream::connect(socket).await.unwrap();
        s.write_all(
            format!(
                "POST /token HTTP/1.1\r\nhost: iohr\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out
    }

    fn wait_for(p: &Path) {
        let start = Instant::now();
        while !p.exists() || std::fs::read(p).unwrap().is_empty() {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "{} never appeared",
                p.display()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_extension_runs_with_a_scoped_token_channel() {
        let b = Box_::new().await;
        let key = KeyPair::generate_ecdsa_p256().unwrap();
        b.trust(&key);
        let work = b.dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let a = Artifact::new(
            "agent",
            "1.0.0",
            &["agents:write", "domains:read"],
            &program(&work),
        );
        publish(&b.server, "agent", "1.0.0", &a, Some(&key)).await;
        let o = b.run(&["ext", "install", "agent@1.0.0"]);
        assert_eq!(code(&o), 0, "{}", text(&o));

        let t = token(&["agents:write", "identity:read"]);
        let child = b
            .cmd(&["agent", "run", "--flag"])
            .env("IOHR_TOKEN", &t)
            .env("IOHR_TOKEN_CI", &t)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        wait_for(&work.join("env.txt"));
        let env = std::fs::read_to_string(work.join("env.txt")).unwrap();
        assert!(
            !env.contains("TOKENSIGNATUREMARKER"),
            "the extension saw a token: {env}"
        );
        assert!(env.contains("IOHR_EXT_API=http://127.0.0.1:9"), "{env}");
        assert_eq!(
            std::fs::read_to_string(work.join("args.txt"))
                .unwrap()
                .trim(),
            "run --flag"
        );
        let socket = env
            .lines()
            .find_map(|l| l.strip_prefix("IOHR_EXT_TOKEN_SOCKET="))
            .unwrap()
            .to_owned();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(Path::new(&socket)), 0o600);
        assert_eq!(mode(Path::new(&socket).parent().unwrap()), 0o700);

        let ok = ask(&socket, r#"{"scopes":["agents:write"]}"#).await;
        assert!(ok.starts_with("HTTP/1.1 200") && ok.contains(&t), "{ok}");
        assert!(
            ok.contains("\"expires_at\":\"2100-01-01T00:00:00Z\""),
            "{ok}"
        );
        // Declared, but the profile's credential does not hold it.
        let not_held = ask(&socket, r#"{"scopes":["domains:read"]}"#).await;
        assert!(
            not_held.starts_with("HTTP/1.1 403") && !not_held.contains(&t),
            "{not_held}"
        );
        // Not declared in the manifest.
        let undeclared = ask(&socket, r#"{"scopes":["tokens:write"]}"#).await;
        assert!(undeclared.contains("scope_not_declared"), "{undeclared}");

        std::fs::write(work.join("release"), b"").unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(7), "{}", text(&out));
        assert!(!text(&out).contains("TOKENSIGNATUREMARKER"));
        assert!(
            !Path::new(&socket).parent().unwrap().exists(),
            "the channel was left behind"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_program_changed_on_disk_does_not_run() {
        let b = Box_::new().await;
        let key = KeyPair::generate_ecdsa_p256().unwrap();
        b.trust(&key);
        let a = Artifact::new("agent", "1.0.0", &[], b"#!/bin/sh\nexit 0\n");
        publish(&b.server, "agent", "1.0.0", &a, Some(&key)).await;
        assert_eq!(code(&b.run(&["ext", "install", "agent@1.0.0"])), 0);
        let dir = std::fs::read_dir(b.data().join("agent"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        std::fs::write(dir.join("prog"), b"#!/bin/sh\necho owned\n").unwrap();
        let o = b.run(&["agent"]);
        assert_eq!(code(&o), 1, "{}", text(&o));
        assert!(
            text(&o).contains("changed since it was installed"),
            "{}",
            text(&o)
        );
        let o = b.run(&["ext", "verify", "agent"]);
        assert_eq!(code(&o), 1, "{}", text(&o));
        assert!(text(&o).contains("FAILED"), "{}", text(&o));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unknown_command_says_how_to_install_it() {
        let b = Box_::new().await;
        let o = b.run(&["agent", "status"]);
        assert_eq!(code(&o), 2, "{}", text(&o));
        assert!(text(&o).contains("iohr ext install agent"), "{}", text(&o));
    }
}
