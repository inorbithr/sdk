//! Extensions end to end against an in-process OCI registry: install verifies digest
//! and signature, refuses tampering and strangers, pins a lock, syncs from it, and runs
//! the program with a token channel that hands out only declared scopes.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

// The pty helper is unix only; only the unix `init_crawl` tests use it.
#[cfg(unix)]
mod pty;
mod support;

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use support::kept_env;

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
        Self::from_config(
            &config,
            "application/vnd.inorbit.iohr.extension.config.v1+json",
            program,
        )
    }

    /// An artifact whose manifest declares `host_actions`, under the config media type
    /// `config_type`.
    fn acting(name: &str, host_actions: &serde_json::Value, config_type: &str) -> Self {
        let config = serde_json::json!({
            "name": name, "version": "1.0.0", "entrypoint": "bin/prog",
            "scopes": [], "description": "a test extension",
            "host_actions": host_actions,
        });
        Self::from_config(&config, config_type, b"prog")
    }

    fn from_config(config: &serde_json::Value, config_type: &str, program: &[u8]) -> Self {
        let config = config.to_string().into_bytes();
        let layer = tar_gz(&[("README", b'0', b"hi"), ("bin/prog", b'0', program)]);
        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "config": {"mediaType": config_type,
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
            .envs(kept_env())
            .current_dir(self.dir.path());
        if let Some(v) = std::env::var_os("HOME") {
            c.env("HOME", v);
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

fn restart_action() -> serde_json::Value {
    serde_json::json!([{
        "id": "agent.restart-worker",
        "params": {"worker": {"type": "dns-label"}},
        "os": ["linux", "macos", "windows"],
        "effect": "Restarts worker {worker}.",
        "timeout_secs": 60,
    }])
}

#[tokio::test(flavor = "multi_thread")]
async fn host_actions_install_only_from_a_v2_config_and_are_shown() {
    let b = Box_::new().await;
    let key = KeyPair::generate_ecdsa_p256().unwrap();
    b.trust(&key);
    let v1 = Artifact::acting(
        "agent",
        &restart_action(),
        "application/vnd.inorbit.iohr.extension.config.v1+json",
    );
    publish(&b.server, "agent", "1.0.0", &v1, Some(&key)).await;
    let o = b.run(&["ext", "install", "agent@1.0.0"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("only a v2 config"), "{}", text(&o));
    nothing_installed(&b);

    let v2 = Artifact::acting(
        "worker",
        &serde_json::json!([{
            "id": "worker.restart",
            "params": {"name": {"type": "dns-label"}},
            "os": ["linux", "macos", "windows"],
            "effect": "Restarts {name}.",
            "timeout_secs": 60,
        }]),
        "application/vnd.inorbit.iohr.extension.config.v2+json",
    );
    publish(&b.server, "worker", "1.0.0", &v2, Some(&key)).await;
    let o = b.run(&["ext", "install", "worker@1.0.0"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(
        text(&o).contains("host actions  worker.restart (Restarts {name}.)"),
        "{}",
        text(&o)
    );
    let o = b.run(&["ext", "list", "--json"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert_eq!(
        stdout_json(&o)[0]["host_actions"][0]["id"],
        "worker.restart",
        "{}",
        text(&o)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_action_outside_the_rules_is_refused_at_install() {
    let b = Box_::new().await;
    let key = KeyPair::generate_ecdsa_p256().unwrap();
    b.trust(&key);
    let mut bad = restart_action();
    bad[0]["params"]["worker"] = serde_json::json!({"type": "path"});
    let a = Artifact::acting(
        "agent",
        &bad,
        "application/vnd.inorbit.iohr.extension.config.v2+json",
    );
    publish(&b.server, "agent", "1.0.0", &a, Some(&key)).await;
    let o = b.run(&["ext", "install", "agent@1.0.0"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("not valid"), "{}", text(&o));
    nothing_installed(&b);
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

/// A catalogue listing of InOrbit's, released (`latest` given) or coming.
fn first_party(name: &str, latest: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "publisher": "inorbit", "name": name, "kind": "program", "description": "d",
        "visibility": "public", "coming": latest.is_none(), "publisher_name": "InOrbit",
        "first_party": true, "latest": latest.map(|v| serde_json::json!({"version": v}))
    })
}

async fn list_catalogue(b: &Box_, listings: &[serde_json::Value]) {
    Mock::given(method("GET"))
        .and(path("/v1/extensions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "extensions": listings, "next_page_token": ""
        })))
        .mount(&b.server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn show_with_no_name_lists_the_catalogue_the_way_search_does() {
    let b = Box_::new().await;
    list_catalogue(
        &b,
        &[
            first_party("agent", Some("0.2.0")),
            first_party("crawler", None),
        ],
    )
    .await;
    let shown = b.run_api(&["ext", "show"]);
    assert_eq!(code(&shown), 0, "{}", text(&shown));
    let out = String::from_utf8_lossy(&shown.stdout);
    assert!(out.starts_with("NAME"), "{out}");
    assert!(
        out.contains("inorbit/agent") && out.contains("0.2.0"),
        "{out}"
    );
    let crawler = out
        .lines()
        .find(|l| l.contains("inorbit/crawler"))
        .unwrap_or_default();
    assert!(crawler.contains("coming"), "{out}");
    // The one listing: the bytes `ext search` prints.
    let searched = b.run_api(&["ext", "search"]);
    assert_eq!(shown.stdout, searched.stdout, "{}", text(&searched));
}

#[tokio::test(flavor = "multi_thread")]
async fn init_without_a_terminal_hints_the_extension_and_installs_nothing() {
    let b = Box_::new().await;
    list_catalogue(
        &b,
        &[
            first_party("agent", Some("0.2.0")),
            first_party("crawler", None),
        ],
    )
    .await;
    let dir = b.dir.path().join("work/made");
    let o = b
        .cmd(&[
            "project",
            "init",
            "made",
            "--template",
            "coordinated",
            "--set",
            "license=MIT",
            "--yes",
            "--no-smoke",
            "--dir",
            dir.to_str().unwrap(),
        ])
        .env("HOME", b.dir.path())
        .env("IOHR_BASE_URL", b.server.uri())
        .env("IOHR_TOKEN", token(&["extensions:read"]))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(code(&o), 0, "{}", text(&o));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("ext install inorbit/agent"), "{}", text(&o));
    assert!(!err.contains("ext install inorbit/crawler"), "{}", text(&o));
    nothing_installed(&b);
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
        // The program writes env.txt, then args.txt: wait for both, or a slow runner
        // reads args.txt while it is still empty.
        wait_for(&work.join("env.txt"));
        wait_for(&work.join("args.txt"));
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
        assert!(
            text(&o).contains("not an iohr command or an installed extension")
                && text(&o).contains("iohr ext install PUBLISHER/NAME"),
            "{}",
            text(&o)
        );
        // The word typed is not repeated (the examples say "AI agent"; the command was `agent`).
        assert!(!text(&o).contains("`agent`"), "{}", text(&o));
        assert!(!text(&o).contains("iohr agent"), "{}", text(&o));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn packages_lists_published_private_shared_local_and_installed() {
    let b = Box_::new().await;
    let home = b.dir.path().join("home");
    for d in ["wip", "Not A Name"] {
        std::fs::create_dir_all(home.join(".inorbit/ext-dev").join(d)).unwrap();
    }
    std::fs::write(home.join(".inorbit/ext-dev/afile"), b"x").unwrap();
    let ext = |p: &str, n: &str, vis: &str, rel: &str| {
        serde_json::json!({
            "publisher": p, "name": n, "kind": "program", "description": "d",
            "visibility": vis, "relation": rel, "publisher_name": "Acme", "first_party": false,
            "latest": {"version": "1.2.0"}
        })
    };
    // Only answers the `mine` form: an unfiltered list would not carry the private ones.
    Mock::given(method("GET"))
        .and(path("/v1/extensions"))
        .and(wiremock::matchers::query_param("mine", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "extensions": [
                ext("acme", "billing", "private", "own"),
                ext("acme", "open", "public", "own"),
                ext("zeta", "partner", "private", "shared"),
            ],
            "next_page_token": ""
        })))
        .mount(&b.server)
        .await;
    let run = |args: &[&str]| {
        b.cmd(args)
            // The home directory is HOME on unix and USERPROFILE on Windows.
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("IOHR_BASE_URL", b.server.uri())
            .env("IOHR_TOKEN", token(&["extensions:read"]))
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };

    let o = run(&["ext", "packages"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(
        out.starts_with("NAME") && out.contains("SOURCE") && out.contains("INSTALLED"),
        "{out}"
    );
    assert!(
        out.contains("acme/billing  program  account  private     1.2.0    no"),
        "{out}"
    );
    assert!(
        out.contains("acme/open     program  account  public"),
        "{out}"
    );
    assert!(
        out.contains("zeta/partner  program  shared   private"),
        "{out}"
    );
    assert!(out.contains("wip") && out.contains("local"), "{out}");
    assert!(
        !out.contains("Not A Name") && !out.contains("afile"),
        "{out}"
    );

    // The top-level alias is the same command, and --json is an array of the same rows.
    let o = run(&["packages", "--json"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let j = stdout_json(&o);
    let rows = j.as_array().unwrap();
    assert_eq!(rows.len(), 4, "{j}");
    let sources: Vec<&str> = rows.iter().map(|r| r["source"].as_str().unwrap()).collect();
    assert_eq!(sources, ["account", "account", "shared", "local"]);
    assert_eq!(rows[0]["name"], "acme/billing");
    assert_eq!(rows[0]["visibility"], "private");
    assert!(rows[0]["installed"].is_null());
    assert_eq!(rows[3]["visibility"], "local");
}

#[tokio::test(flavor = "multi_thread")]
async fn packages_still_lists_this_machine_when_the_api_refuses() {
    let b = Box_::new().await;
    let home = b.dir.path().join("home");
    std::fs::create_dir_all(home.join(".inorbit/ext-dev/wip")).unwrap();
    Mock::given(method("GET"))
        .and(path("/v1/extensions"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "code": 7, "message": "denied"
        })))
        .mount(&b.server)
        .await;
    let o = b
        .cmd(&["ext", "packages", "--json"])
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("IOHR_BASE_URL", b.server.uri())
        .env("IOHR_TOKEN", token(&["extensions:read"]))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_ne!(code(&o), 0, "{}", text(&o));
    assert!(text(&o).contains("extensions:read"), "{}", text(&o));
    let j = stdout_json(&o);
    assert_eq!(j[0]["name"], "wip");
    assert_eq!(j[0]["source"], "local");
}

/// Runs `iohr ext verify TARGET --bundle BUNDLE --publisher-key KEY`, with `--json` when asked.
fn verify(b: &Box_, target: &str, bundle: &str, key: &str, json: bool) -> Output {
    let mut args = vec![
        "ext",
        "verify",
        target,
        "--bundle",
        bundle,
        "--publisher-key",
        key,
    ];
    if json {
        args.push("--json");
    }
    b.run(&args)
}

/// `iohr ext verify ARTIFACT --bundle FILE --publisher-key PEM` (RFC 0143): the install's
/// trust check on one artifact, offline, no sign-in.
#[tokio::test(flavor = "multi_thread")]
async fn verifying_an_artifact_accepts_a_good_bundle_and_refuses_a_tampered_one() {
    let b = Box_::new().await;
    let key = KeyPair::generate_ecdsa_p256().unwrap();
    let stranger = KeyPair::generate_ecdsa_p256().unwrap();
    let artifact = b.dir.path().join("todo.tar.gz");
    std::fs::write(&artifact, b"the program").unwrap();
    let digest = Digest::of(b"the program");
    let (art, pem, good) = (
        artifact.to_str().unwrap().to_owned(),
        b.dir.path().join("publisher.pem"),
        b.dir.path().join("good.sigstore.json"),
    );
    std::fs::write(&pem, key.public_key_der().unwrap().to_pem()).unwrap();
    std::fs::write(&good, bundle(&key, &digest)).unwrap();
    let (pem_s, good_s) = (pem.to_str().unwrap(), good.to_str().unwrap());

    // A file, then its digest, both pass; the signer is the publisher's key.
    for target in [art.as_str(), digest.to_string().as_str()] {
        let o = verify(&b, target, good_s, pem_s, true);
        assert_eq!(code(&o), 0, "{}", text(&o));
        let j = stdout_json(&o);
        assert_eq!(j["ok"], true);
        assert_eq!(j["digest"], digest.to_string());
        assert_eq!(j["signer"], fingerprint(&key));
    }
    let o = verify(&b, &art, good_s, pem_s, false);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(String::from_utf8_lossy(&o.stdout).contains(&fingerprint(&key)));

    // The artifact changed after signing: the bundle names another digest.
    std::fs::write(&artifact, b"the program, tampered").unwrap();
    let o = verify(&b, &art, good_s, pem_s, true);
    assert_ne!(code(&o), 0, "{}", text(&o));
    let j = stdout_json(&o);
    assert_eq!(j["ok"], false);
    assert!(
        j["error"]
            .as_str()
            .unwrap()
            .contains("does not name this artifact"),
        "{j}"
    );
    assert!(j["signer"].is_null());

    // The bundle changed after signing: one flipped byte of the signature.
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&good).unwrap()).unwrap();
    let sig = v["dsseEnvelope"]["signatures"][0]["sig"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut raw = STANDARD.decode(&sig).unwrap();
    let last = raw.len() - 1;
    raw[last] ^= 0xff;
    v["dsseEnvelope"]["signatures"][0]["sig"] = STANDARD.encode(raw).into();
    let bad = b.dir.path().join("bad.sigstore.json");
    std::fs::write(&bad, v.to_string()).unwrap();
    std::fs::write(&artifact, b"the program").unwrap();
    let o = verify(&b, &art, bad.to_str().unwrap(), pem_s, false);
    assert_ne!(code(&o), 0, "{}", text(&o));
    assert!(text(&o).contains("is not trusted"), "{}", text(&o));

    // A signer the publisher key does not name.
    let other = b.dir.path().join("stranger.pem");
    std::fs::write(&other, stranger.public_key_der().unwrap().to_pem()).unwrap();
    let o = verify(&b, &art, good_s, other.to_str().unwrap(), false);
    assert_ne!(code(&o), 0, "{}", text(&o));

    // No bundle, and a missing file, are usage errors, not a pass.
    let o = b.run(&["ext", "verify", &art, "--publisher-key", pem_s]);
    assert_ne!(code(&o), 0);
    let o = b.run(&["ext", "verify", "/no/such/file", "--bundle", good_s]);
    assert_ne!(code(&o), 0);
}

/// `project init` reads a public site on this machine through the agent extension (PRD
/// 0190.1): consent first, the files under `.inorbit/crawl/<host>/`, a study the sync lists
/// and a memory entry, and nothing sent anywhere.
#[cfg(unix)]
mod init_crawl {
    use std::fmt::Write as _;
    use std::path::Path;

    use super::*;

    const SPACE: &str = "lab_1";

    /// A stand-in for `iohr-agent`: asks the site for its start page (so the test counts the
    /// requests), writes the crawler's JSON to `--out`, records its arguments beside it and
    /// exits with `exit`.
    fn fake_agent(exit: u8, stopped_by: &[&str], json: bool) -> Vec<u8> {
        fake_agent_with(exit, stopped_by, json, &serde_json::json!([]))
    }

    /// The same, with `blocked` as the links the crawler met and kept out.
    fn fake_agent_with(
        exit: u8,
        stopped_by: &[&str],
        json: bool,
        blocked: &serde_json::Value,
    ) -> Vec<u8> {
        let result = serde_json::json!({
            "start": "http://site.test/", "scope": "site.test",
            "complete": stopped_by.is_empty(), "stopped_by": stopped_by,
            "pages": [
                {"url": "http://site.test/", "status": 200, "title": "Home",
                 "content_type": "text/html", "size": 10, "depth": 0, "rendered": false},
                {"url": "http://site.test/about", "status": 200, "title": "About us",
                 "content_type": "text/html", "size": 20, "depth": 1, "rendered": false},
                {"url": "http://site.test/gone", "status": 404, "content_type": "text/html",
                 "size": 0, "depth": 1, "rendered": false}
            ],
            "hosts": [{"host": "site.test", "robots": "rules", "pages": 3, "disallowed": 2,
                       "media_skipped": 0, "refused": false}],
            "blocked": blocked, "links": [], "listings": [],
            "used": {"pages": 3, "hosts": 1, "bytes": 30, "seconds": 3, "requests": 4, "renders": 0}
        })
        .to_string();
        let write = if json {
            format!("printf '%s' '{result}' > \"$out\"\n")
        } else {
            String::new()
        };
        format!(
            "#!/bin/sh\nall=\"$*\"\nurl=\"$3\"\nwhile [ $# -gt 0 ]; do\n  \
             if [ \"$1\" = \"--out\" ]; then out=\"$2\"; fi\n  shift\ndone\n\
             curl -s -o /dev/null \"$url\"\ndir=$(dirname \"$out\")\n\
             echo \"$all\" > \"$dir/args.txt\"\n{write}exit {exit}\n"
        )
        .into_bytes()
    }

    struct Site {
        b: Box_,
        server: MockServer,
        dir: PathBuf,
    }

    /// A box with the fake agent installed (or not) and a site that must be asked `hits`
    /// times (a mock that is asked a different number of times fails the test when dropped).
    async fn site(agent: Option<Vec<u8>>, hits: std::ops::RangeInclusive<u64>) -> Site {
        let b = Box_::new().await;
        if let Some(program) = agent {
            let key = KeyPair::generate_ecdsa_p256().unwrap();
            b.trust(&key);
            let a = Artifact::new("agent", "1.0.0", &["agents:write"], &program);
            publish(&b.server, "agent", "1.0.0", &a, Some(&key)).await;
            let o = b.run(&["ext", "install", "agent@1.0.0"]);
            assert_eq!(code(&o), 0, "{}", text(&o));
        }
        let site = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<title>Home</title>"))
            .expect(hits)
            .mount(&site)
            .await;
        let dir = b.dir.path().join("work/made");
        Site {
            b,
            server: site,
            dir,
        }
    }

    impl Site {
        fn init(&self, extra: &[&str]) -> Output {
            self.init_env(extra, &[])
        }

        /// The same, with `env` set on the command beside the rest.
        fn init_env(&self, extra: &[&str], env: &[(&str, &str)]) -> Output {
            let t = token(&["extensions:read", "agents:write"]);
            let mut args = vec![
                "project",
                "init",
                "made",
                "--template",
                "coordinated",
                "--set",
                "license=MIT",
                "--yes",
                "--no-smoke",
                "--no-sync",
                "--dir",
                self.dir.to_str().unwrap(),
            ];
            args.extend_from_slice(extra);
            let mut c = self.b.cmd(&args);
            c.env("HOME", self.b.dir.path())
                .env("IOHR_BASE_URL", self.b.server.uri())
                .env("IOHR_TOKEN", &t)
                .env("IOHR_TOKEN_CI", &t)
                .stdin(Stdio::null());
            for (k, v) in env {
                c.env(k, v);
            }
            c.output().unwrap()
        }

        /// The folder a crawl of this site keeps its files in.
        fn crawl_dir(&self) -> PathBuf {
            let addr = self.server.address();
            self.dir
                .join(format!(".inorbit/crawl/{}-{}", addr.ip(), addr.port()))
        }

        fn url(&self) -> String {
            format!("{}/", self.server.uri())
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_crawl_and_no_terminal_read_nothing() {
        // No flag and no terminal: no consent, so no request and no crawl folder.
        let s = site(Some(fake_agent(0, &[], true)), 0..=0).await;
        let o = s.init(&[]);
        assert_eq!(code(&o), 0, "{}", text(&o));
        assert!(!s.dir.join(".inorbit/crawl").exists(), "{}", text(&o));
        assert!(!text(&o).contains("Crawl"), "{}", text(&o));
        // Even with a URL in the flag's place, --no-crawl wins and the two do not mix.
        let o = s.init(&["--no-crawl"]);
        assert_eq!(code(&o), 0, "{}", text(&o));
        assert!(!s.dir.join(".inorbit/crawl").exists());
        let o = s.init(&["--no-crawl", "--crawl", &s.url()]);
        assert_eq!(code(&o), 2, "{}", text(&o));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn crawl_keeps_the_files_a_study_and_a_memory_and_sends_nothing() {
        let s = site(Some(fake_agent(0, &[], true)), 1..=1).await;
        let o = s.init(&["--crawl", &s.url()]);
        let t = text(&o);
        assert_eq!(code(&o), 0, "{t}");
        // Consent first, then the summary.
        let consent = t.find("Nothing is sent to InOrbit").expect(&t);
        let said = t
            .find("read 2 pages in 3 s, 2 skipped by robots.txt, 1 failed")
            .expect(&t);
        assert!(consent < said, "{t}");
        assert!(t.contains("stopped: no limit reached"), "{t}");
        assert!(
            t.contains("Nothing was sent to InOrbit. Run iohr sync when you want it there."),
            "{t}"
        );
        // The raw record, under the host's folder, with the policy it ran under.
        let dirc = s.crawl_dir();
        assert!(dirc.join("crawl.json").is_file(), "{t}");
        assert_eq!(
            std::fs::read_to_string(dirc.join("facts.jsonl"))
                .unwrap()
                .lines()
                .count(),
            3
        );
        let policy = std::fs::read_to_string(dirc.join("policy.toml")).unwrap();
        assert!(policy.contains("allow = [\"127.0.0.1\"]"), "{policy}");
        let args = std::fs::read_to_string(dirc.join("args.txt")).unwrap();
        assert!(
            args.contains("--max-pages 200") && args.contains("--max-minutes 5"),
            "{args}"
        );
        assert!(
            args.starts_with(&format!("atlas crawl {}", s.url())),
            "{args}"
        );
        // The raw record stays out of the project's git.
        assert_eq!(
            std::fs::read_to_string(s.dir.join(".inorbit/crawl/.gitignore")).unwrap(),
            "*\n"
        );
        // The study the sync carries, and one memory entry that points at it.
        let study = s
            .dir
            .read_dir_of("atlas/studies")
            .into_iter()
            .find(|n| n.contains("site-crawl"))
            .expect("a site crawl study");
        let text_of = std::fs::read_to_string(s.dir.join("atlas/studies").join(&study)).unwrap();
        assert!(text_of.contains("http://site.test/about"), "{text_of}");
        assert_eq!(
            crawl_memories(&s.dir),
            1,
            "one memory entry points at the crawl"
        );
        // `iohr sync --dry-run` lists the study and sends nothing.
        let o =
            s.b.cmd(&["sync", "--dry-run", "--space", SPACE])
                .env("HOME", s.b.dir.path())
                .current_dir(&s.dir)
                .stdin(Stdio::null())
                .output()
                .unwrap();
        assert_eq!(code(&o), 0, "{}", text(&o));
        assert!(
            text(&o).contains(&format!("atlas/studies/{study}")),
            "{}",
            text(&o)
        );
        // Nothing the crawl found went to the InOrbit server of the test.
        let seen = s.b.server.received_requests().await.unwrap_or_default();
        // Init's own plan record is the one write; no atlas or decisions path,
        // and no body carries a crawled page.
        assert!(
            seen.iter().all(|r| {
                let path = r.url.path();
                !path.contains("atlas")
                    && !path.contains("decisions")
                    && !String::from_utf8_lossy(&r.body).contains("site.test")
                    && (r.method.as_str() == "GET" || path == "/v1/coordinator/plans")
            }),
            "{seen:?}"
        );
    }

    /// The fake agent, that also leaves the HTML of the pages it read in `pages/` beside its
    /// result when it is told `--keep-pages`, under the names the real one gives them.
    fn agent_that_keeps_pages() -> Vec<u8> {
        let pages = [
            (
                "http://site.test/",
                "<h1>Home</h1><p>Site Test builds warehouse picking robots for logistics teams.</p>",
            ),
            (
                "http://site.test/about",
                "<h1>About us</h1><p>Write to hello@site.test about the Picker robot, which costs $1,299.00 per month.</p>",
            ),
        ];
        let mut writes = String::new();
        for (url, body) in pages {
            let name = iohr::kb::fetch::cache_path(Path::new(""), url);
            let _ = writeln!(
                writes,
                "  printf '%s' '<html><head><title>T</title></head><body><main>{body}</main></body></html>' > \"$dir/pages/{}\"",
                name.file_name().unwrap().to_string_lossy()
            );
        }
        String::from_utf8(fake_agent(0, &[], true))
            .unwrap()
            .replacen(
                "echo \"$all\" > \"$dir/args.txt\"\n",
                &format!("echo \"$all\" > \"$dir/args.txt\"\ncase \"$all\" in *--keep-pages*)\n  mkdir -p \"$dir/pages\"\n{writes};;\nesac\n"),
                1,
            )
            .into_bytes()
    }

    /// An agent that knows `--rate` but not `--keep-pages`.
    fn agent_without_keep_pages() -> Vec<u8> {
        String::from_utf8(fake_agent(0, &[], true))
            .unwrap()
            .replacen(
                "all=\"$*\"\n",
                "all=\"$*\"\ncase \"$all\" in *--keep-pages*)\n  \
                 echo \"error: unexpected argument '--keep-pages' found\" >&2\n  exit 2;;\nesac\n",
                1,
            )
            .into_bytes()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_kept_pages_become_a_knowledge_base_with_no_second_fetch_and_one_study() {
        let s = site(Some(agent_that_keeps_pages()), 1..=1).await;
        let o = s.init(&["--crawl", &s.url()]);
        let t = text(&o);
        assert_eq!(code(&o), 0, "{t}");
        let args = std::fs::read_to_string(s.crawl_dir().join("args.txt")).unwrap();
        assert!(args.trim_end().ends_with("--keep-pages"), "{args}");
        assert!(
            t.contains("page text: 2 from files on this machine, 0 requested now"),
            "{t}"
        );
        // The model is in the crawl's own facts and its one study; no model file, no second study.
        let dirc = s.crawl_dir();
        let facts = std::fs::read_to_string(dirc.join("facts.jsonl")).unwrap();
        assert!(
            facts.contains("\"kind\":\"page\"") && facts.contains("\"kind\":\"entity\""),
            "{facts}"
        );
        assert!(!dirc.join("model.jsonl").exists());
        let studies: Vec<String> = s
            .dir
            .read_dir_of("atlas/studies")
            .into_iter()
            .filter(|n| n.contains("site-crawl"))
            .collect();
        assert_eq!(
            studies.len(),
            1,
            "one crawl study, no separate model study: {studies:?}"
        );
        let study = std::fs::read_to_string(s.dir.join("atlas/studies").join(&studies[0])).unwrap();
        assert!(
            study.contains("## Site model") && study.contains("## Pages"),
            "{study}"
        );
        // The index answers, with the page it came from.
        let o =
            s.b.cmd(&["atlas", "search", "how much does the picker cost", "--json"])
                .env("HOME", s.b.dir.path())
                .current_dir(&s.dir)
                .stdin(Stdio::null())
                .output()
                .unwrap();
        assert_eq!(code(&o), 0, "{}", text(&o));
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        assert!(
            v["hits"][0]["url"].as_str().unwrap().ends_with("/about"),
            "{v}"
        );
        assert!(
            v["hits"][0]["text"].as_str().unwrap().contains("$1,299.00"),
            "{v}"
        );
        // The site was asked once, by the agent (the mock fails on any other count), and a
        // second init over the same files makes no request for them either.
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_without_keep_pages_is_run_again_and_iohr_asks_only_inside_the_scope() {
        let s = site(Some(agent_without_keep_pages()), 1..=1).await;
        let o = s.init(&["--crawl", &s.url()]);
        let t = text(&o);
        assert_eq!(code(&o), 0, "{t}");
        assert!(!t.contains("the crawler exited"), "{t}");
        let args = std::fs::read_to_string(s.crawl_dir().join("args.txt")).unwrap();
        assert!(
            args.contains("--rate") && !args.contains("--keep-pages"),
            "the second run kept its rate and dropped the flag: {args}"
        );
        // The fake agent's pages are on site.test, outside the scope of the 127.0.0.1 crawl:
        // none is requested, and the box says so.
        assert!(t.contains("0 requested now"), "{t}");
        assert!(
            t.contains("not requested (outside the scope or robots.txt)"),
            "{t}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn init_json_stays_one_document_with_the_knowledge_base_in_the_crawl_step() {
        let s = site(Some(agent_that_keeps_pages()), 1..=1).await;
        let o = s.init(&["--crawl", &s.url(), "--json"]);
        assert_eq!(code(&o), 0, "{}", text(&o));
        let out = String::from_utf8_lossy(&o.stdout).into_owned();
        let v: serde_json::Value =
            serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("{e}: {out}"));
        assert!(out.contains("knowledge base: "), "{out}");
        assert!(
            !out.contains("Reading the text"),
            "no spinner in json: {out}"
        );
        let _ = v;
    }

    /// The progress line the agent writes under `--progress` (RFC 0163): `total` only once a
    /// sitemap has said how much there is.
    fn progress_line(read: u32, queued: u32, total: Option<u32>) -> String {
        let total = total.map_or_else(String::new, |t| format!(",\"total\":{t}"));
        format!(
            "{{\"event\":\"progress\",\"v\":1,\"scope\":\"site.test\",\"phase\":\"pages\",\"read\":{read},\"failed\":0,\"queued\":{queued}{total},\"cap\":200}}"
        )
    }

    /// The fake agent, that first writes `lines` to standard error `gap` seconds apart, as a
    /// crawl that takes a while would.
    fn agent_that_reports(lines: &[String], gap: u32) -> Vec<u8> {
        let said: String = lines
            .iter()
            .map(|l| format!("echo '{l}' >&2\nsleep {gap}\n"))
            .collect::<Vec<_>>()
            .concat();
        String::from_utf8(fake_agent(0, &[], true))
            .unwrap()
            .replacen("curl -s", &format!("{said}curl -s"), 1)
            .into_bytes()
    }

    /// An agent from before RFC 0163: clap's refusal of the flags it does not know, then the
    /// same crawl when run without them.
    fn old_agent() -> Vec<u8> {
        String::from_utf8(fake_agent(0, &[], true))
            .unwrap()
            .replacen(
                "all=\"$*\"\n",
                "all=\"$*\"\ncase \"$all\" in *--rate*|*--progress*)\n  \
                 echo \"error: unexpected argument '--rate' found\" >&2\n  exit 2;;\nesac\n",
                1,
            )
            .into_bytes()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_rate_is_ten_a_second_by_default_and_the_agent_is_told() {
        let s = site(Some(fake_agent(0, &[], true)), 1..=1).await;
        let o = s.init(&["--crawl", &s.url()]);
        let t = text(&o);
        assert_eq!(code(&o), 0, "{t}");
        assert!(
            t.contains("at most 10 requests a second to one host"),
            "the consent says the rate: {t}"
        );
        let args = std::fs::read_to_string(s.crawl_dir().join("args.txt")).unwrap();
        assert!(
            args.trim_end()
                .ends_with("--rate 10 --progress --keep-pages"),
            "{args}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_rate_is_the_flag_or_the_environment_and_stays_in_bounds() {
        let s = site(Some(fake_agent(0, &[], true)), 2..=2).await;
        let o = s.init(&["--crawl", &s.url(), "--crawl-rate", "3"]);
        let t = text(&o);
        assert_eq!(code(&o), 0, "{t}");
        assert!(t.contains("at most 3 requests a second"), "{t}");
        let args = std::fs::read_to_string(s.crawl_dir().join("args.txt")).unwrap();
        assert!(args.contains("--rate 3 "), "{args}");
        // The flag is also a variable of the same name, and the flag wins over it.
        let o = s.init_env(
            &["--crawl", &s.url(), "--crawl-rate", "4"],
            &[("IOHR_CRAWL_RATE", "9")],
        );
        assert!(text(&o).contains("at most 4 requests"), "{}", text(&o));
        // Nothing outside 1 to 50 is accepted, and nothing is read.
        for bad in ["0", "51", "fast"] {
            let o = s.init(&["--crawl", &s.url(), "--crawl-rate", bad]);
            assert_eq!(code(&o), 2, "{bad}: {}", text(&o));
        }
        let o = s.init_env(&["--crawl", &s.url()], &[("IOHR_CRAWL_RATE", "fast")]);
        assert_eq!(code(&o), 2, "a bad variable with a crawl: {}", text(&o));
        assert!(text(&o).contains("1 to 50"), "{}", text(&o));
    }

    /// A bad `IOHR_CRAWL_RATE` is only a problem for a crawl: `--no-crawl` never reads it, and
    /// nothing is crawled when no site was given.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_bad_rate_in_the_environment_does_not_stop_init_without_a_crawl() {
        let s = site(Some(fake_agent(0, &[], true)), 0..=0).await;
        for extra in [&["--no-crawl"][..], &[][..]] {
            let o = s.init_env(extra, &[("IOHR_CRAWL_RATE", "fast")]);
            assert_eq!(code(&o), 0, "{extra:?}: {}", text(&o));
            assert!(!s.crawl_dir().exists(), "{extra:?}: nothing was crawled");
            std::fs::remove_dir_all(&s.dir).unwrap();
            std::fs::create_dir_all(&s.dir).unwrap();
        }
    }

    /// No bar is drawn for a script: with no terminal (a test's pipe), with `--json`, with
    /// `NO_COLOR`, the progress an agent reports is read and dropped, and what is written has
    /// no progress text, escape, rail or bar character.
    #[tokio::test(flavor = "multi_thread")]
    async fn no_bar_is_drawn_with_json_or_no_color() {
        let lines = [progress_line(1, 4, None), progress_line(2, 3, Some(5))];
        let glyphs = |c: char| c == '\u{1b}' || ('\u{2500}'..='\u{25ff}').contains(&c) || c == '\r';
        for (name, extra, env) in [
            ("no_terminal", &[][..], &[][..]),
            ("json", &["--json"][..], &[][..]),
            (
                "no_color",
                &[][..],
                &[("NO_COLOR", "1"), ("TERM", "xterm-256color")][..],
            ),
            (
                "no_color_and_json",
                &["--json"][..],
                &[("NO_COLOR", "1")][..],
            ),
        ] {
            let s = site(Some(agent_that_reports(&lines, 1)), 1..=1).await;
            let mut args = vec!["--crawl".to_owned(), s.url()];
            args.extend(extra.iter().map(|a| (*a).to_owned()));
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let o = s.init_env(&args, env);
            let t = text(&o);
            assert_eq!(code(&o), 0, "{name}: {t}");
            assert!(
                !t.contains("pages read, ")
                    && !t.contains("of 5 pages")
                    && !t.contains("\"event\""),
                "{name}: progress text in the output: {t}"
            );
            assert!(
                !t.chars().any(glyphs),
                "{name}: a bar or an escape in the output: {t:?}"
            );
            // The crawl did run, with progress asked for.
            let said = std::fs::read_to_string(s.crawl_dir().join("args.txt")).unwrap();
            assert!(said.contains("--progress"), "{name}: {said}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_older_agent_is_run_again_without_the_flags_and_the_crawl_still_works() {
        let s = site(Some(old_agent()), 1..=1).await;
        let o = s.init(&["--crawl", &s.url()]);
        let t = text(&o);
        assert_eq!(code(&o), 0, "{t}");
        assert!(t.contains("read 2 pages in 3 s"), "{t}");
        assert!(!t.contains("the crawler exited"), "{t}");
        let args = std::fs::read_to_string(s.crawl_dir().join("args.txt")).unwrap();
        assert!(
            !args.contains("--rate") && !args.contains("--progress"),
            "the second run had neither flag: {args}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_crawl_that_fails_for_another_reason_is_not_run_twice() {
        // Exit 2 without the agent naming a flag is not an old agent: one run, one failure.
        let agent = String::from_utf8(fake_agent(2, &[], false))
            .unwrap()
            .replacen("curl -s", "echo 'error: bad policy' >&2\ncurl -s", 1)
            .into_bytes();
        let s = site(Some(agent), 1..=1).await;
        let o = s.init(&["--crawl", &s.url()]);
        assert_eq!(code(&o), 0, "{}", text(&o));
        assert!(
            text(&o).contains("the crawler exited with 2: error: bad policy"),
            "{}",
            text(&o)
        );
    }

    /// At a terminal the crawl is a spinner with a live count until the agent says how much
    /// there is, then a bar that fills, and the step is gone when it ends.
    #[tokio::test(flavor = "multi_thread")]
    async fn on_a_terminal_a_spinner_counts_until_a_total_then_a_bar_fills() {
        let lines = [
            progress_line(1, 4, None),
            progress_line(2, 3, Some(5)),
            progress_line(4, 1, Some(5)),
        ];
        // The row is drawn at most once a second (`ui::REDRAW`) on a 250 ms tick, so each line
        // must stay longer than that to be seen: a second apart, a state could be passed over.
        let s = site(Some(agent_that_reports(&lines, 2)), 1..=1).await;
        let t = token(&["extensions:read", "agents:write"]);
        let mut cmd = s.b.cmd(&[
            "project",
            "init",
            "made",
            "--template",
            "coordinated",
            "--set",
            "license=MIT",
            "--yes",
            "--no-smoke",
            "--no-sync",
            "--dir",
            s.dir.to_str().unwrap(),
            "--crawl",
            &s.url(),
        ]);
        cmd.env("HOME", s.b.dir.path())
            .env("IOHR_BASE_URL", s.b.server.uri())
            .env("IOHR_TOKEN", &t)
            .env("IOHR_TOKEN_CI", &t)
            .env("TERM", "xterm-256color");
        let mut term = pty::Session::spawn(cmd);
        // No total yet: a count and the queue, no bar.
        term.until("a spinner with a live count", |sc| {
            pty::any(sc, "1 pages read, 4 waiting")
        });
        let reading = (0..pty::ROWS)
            .map(|r| pty::row(term.screen.screen(), r))
            .filter(|r| r.contains("Reading "))
            .collect::<Vec<_>>();
        assert!(
            reading
                .iter()
                .all(|r| !r.contains(" pages, ") && !r.contains(" of ")),
            "no total, so no bar: {reading:?}"
        );
        // A total: pages of it, and what the page limit still allows.
        term.until("a bar with a total", |sc| {
            pty::any(sc, "2 of 5 pages") || pty::any(sc, "4 of 5 pages")
        });
        let shown = term.screen.screen().contents();
        assert!(shown.contains("left to the 200-page limit"), "{shown}");
        let status = term.exited().expect("init ends");
        assert!(status.success(), "{}", term.screen.screen().contents());
        // The bar is gone once the crawl is over; the summary is there.
        let end = term.screen.screen().contents();
        assert!(!end.contains("pages read, 4 waiting"), "{end}");
        assert!(end.contains("Nothing was sent to InOrbit"), "{end}");
    }

    /// The site the real crawler reads: robots.txt, a sitemap that lists `n` pages besides the
    /// start page, and the pages, each request stamped when it arrives.
    struct Timed {
        origin: String,
        pages: usize,
        at: std::sync::Arc<std::sync::Mutex<Vec<(String, std::time::Instant)>>>,
    }

    impl wiremock::Respond for Timed {
        fn respond(&self, r: &Request) -> ResponseTemplate {
            let p = r.url.path().to_owned();
            self.at
                .lock()
                .unwrap()
                .push((p.clone(), std::time::Instant::now()));
            let html = "<html><head><title>T</title></head><body><p>Enough words here to count \
                        as a page with text, so nothing is tried in a browser.</p></body></html>";
            match p.as_str() {
                "/robots.txt" => ResponseTemplate::new(200)
                    .set_body_raw("User-agent: *\nAllow: /\n", "text/plain"),
                "/sitemap.xml" => {
                    let urls: String = (1..=self.pages)
                        .map(|i| format!("<url><loc>{}/p{i}</loc></url>", self.origin))
                        .collect::<Vec<_>>()
                        .concat();
                    ResponseTemplate::new(200).set_body_raw(
                        format!(
                            "<?xml version=\"1.0\"?><urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><url><loc>{}/</loc></url>{urls}</urlset>",
                            self.origin
                        ),
                        "application/xml",
                    )
                }
                _ => ResponseTemplate::new(200).set_body_raw(html, "text/html"),
            }
        }
    }

    /// A box with the real `iohr-agent` (at `IOHR_TEST_AGENT`) installed as the extension, and
    /// a site of `pages` pages.
    async fn real_agent_site(pages: usize) -> (Site, Timed) {
        // The path of an iohr-agent that has `atlas crawl --rate`; unset fails the test here.
        let real = std::env::var("IOHR_TEST_AGENT").unwrap();
        let s = site(
            Some(format!("#!/bin/sh\nexec '{real}' \"$@\"\n").into_bytes()),
            0..=u64::MAX,
        )
        .await;
        let timed = Timed {
            origin: s.server.uri(),
            pages,
            at: std::sync::Arc::default(),
        };
        // The site is the mock the box made, answering through the stamping responder.
        s.server.reset().await;
        Mock::given(method("GET"))
            .respond_with(Timed {
                origin: timed.origin.clone(),
                pages,
                at: timed.at.clone(),
            })
            .mount(&s.server)
            .await;
        (s, timed)
    }

    /// The gaps between the requests for pages (not robots.txt and the sitemap).
    fn page_gaps(t: &Timed) -> Vec<std::time::Duration> {
        let at: Vec<_> =
            t.at.lock()
                .unwrap()
                .iter()
                .filter(|(p, _)| p != "/robots.txt" && p != "/sitemap.xml")
                .map(|(_, i)| *i)
                .collect();
        at.windows(2).map(|w| w[1] - w[0]).collect()
    }

    /// The real crawler under the real binary: on a terminal the bar appears as soon as the
    /// sitemap has said how much there is and fills, and the requests keep to ten a second.
    /// Needs the agent built from the dataplane's RFC 0163 change, so it is run by hand:
    /// `IOHR_TEST_AGENT=<path> cargo test -p iohr --test ext real_agent -- --ignored`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs IOHR_TEST_AGENT, the path of a built iohr-agent"]
    async fn real_agent_a_bar_appears_and_ten_a_second_holds() {
        let (s, timed) = real_agent_site(25).await;
        let t = token(&["extensions:read", "agents:write"]);
        let mut cmd = s.b.cmd(&[
            "project",
            "init",
            "made",
            "--template",
            "coordinated",
            "--set",
            "license=MIT",
            "--yes",
            "--no-smoke",
            "--no-sync",
            "--dir",
            s.dir.to_str().unwrap(),
            "--crawl",
            &s.url(),
        ]);
        cmd.env("HOME", s.b.dir.path())
            .env("IOHR_BASE_URL", s.b.server.uri())
            .env("IOHR_TOKEN", &t)
            .env("IOHR_TOKEN_CI", &t)
            .env("TERM", "xterm-256color");
        let mut term = pty::Session::spawn(cmd);
        // 26 pages: the start page and the 25 the sitemap lists.
        term.until("a bar with the sitemap's total", |sc| {
            pty::any(sc, " of 26 pages")
        });
        let shown = term.screen.screen().contents();
        assert!(shown.contains("left to the 200-page limit"), "{shown}");
        let status = term.exited().expect("init ends");
        let end = term.screen.screen().contents();
        assert!(status.success(), "{end}");
        assert!(end.contains("facts: 26 in atlas/"), "{end}");
        let gaps = page_gaps(&timed);
        assert_eq!(gaps.len(), 25, "26 pages, 25 gaps: {gaps:?}");
        // Ten a second is 100 ms apart; a timer that fires a little early is no breach.
        let least = gaps.iter().min().unwrap();
        assert!(
            *least >= std::time::Duration::from_millis(90),
            "ten a second at most, the closest gap was {least:?}"
        );
        let span: std::time::Duration = gaps.iter().sum();
        assert!(
            span >= std::time::Duration::from_millis(2250),
            "25 gaps of about 100 ms: {span:?}"
        );
    }

    /// The same with the rate the flag names: two a second is 500 ms apart.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs IOHR_TEST_AGENT, the path of a built iohr-agent"]
    async fn real_agent_the_flag_slows_the_crawl() {
        let (s, timed) = real_agent_site(4).await;
        let o = s.init(&["--crawl", &s.url(), "--crawl-rate", "2"]);
        assert_eq!(code(&o), 0, "{}", text(&o));
        assert!(text(&o).contains("read 5 pages"), "{}", text(&o));
        let gaps = page_gaps(&timed);
        assert_eq!(gaps.len(), 4, "{gaps:?}");
        let least = gaps.iter().min().unwrap();
        assert!(
            *least >= std::time::Duration::from_millis(450),
            "two a second at most, the closest gap was {least:?}"
        );
    }

    /// A site that links to a subdomain, which the crawler keeps out by its policy.
    fn agent_that_meets_a_subdomain() -> Vec<u8> {
        fake_agent_with(
            0,
            &[],
            true,
            &serde_json::json!([
                {"url": "http://blog.site.test/", "reason": "policy: host not allowed"},
                {"url": "http://site.test/private", "reason": "robots.txt"}
            ]),
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_subdomain_is_not_followed_without_a_yes() {
        // No terminal (and `--yes`): the one asked-for host is read, the subdomain is not,
        // and the report says which name was left out and how to read it.
        let s = site(Some(agent_that_meets_a_subdomain()), 1..=1).await;
        let o = s.init(&["--crawl", &s.url()]);
        let t = text(&o);
        assert_eq!(code(&o), 0, "{t}");
        assert!(t.contains("not read: blog.site.test"), "{t}");
        assert!(t.contains("--crawl-subdomains"), "{t}");
        assert!(
            !t.contains("and its subdomains"),
            "the consent names the host only: {t}"
        );
        let dirc = s.crawl_dir();
        assert!(!dirc.join("more").exists(), "no second pass ran");
        let policy = std::fs::read_to_string(dirc.join("policy.toml")).unwrap();
        assert!(!policy.contains("blog"), "{policy}");
        let study = s.dir.read_dir_of("atlas/studies");
        let study = study.iter().find(|n| n.contains("site-crawl")).unwrap();
        let study = std::fs::read_to_string(s.dir.join("atlas/studies").join(study)).unwrap();
        assert!(study.contains("were not read: blog.site.test"), "{study}");
        assert_eq!(crawl_memories(&s.dir), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_subdomain_flag_is_the_yes_to_all_and_asks_nothing() {
        let s = site(Some(agent_that_meets_a_subdomain()), 1..=1).await;
        let o = s.init(&["--crawl", &s.url(), "--crawl-subdomains"]);
        let t = text(&o);
        assert_eq!(code(&o), 0, "{t}");
        assert!(t.contains("and its subdomains"), "{t}");
        assert!(!t.contains("not read:"), "{t}");
        assert!(!s.crawl_dir().join("more").exists());
        // The flag and the refusal to crawl cannot be given together.
        let o = s.init(&["--no-crawl", "--crawl-subdomains"]);
        assert_ne!(code(&o), 0, "{}", text(&o));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_budget_stop_is_a_finished_crawl_that_says_how_to_raise_the_limit() {
        let s = site(Some(fake_agent(1, &["pages"], true)), 2..=2).await;
        let o = s.init(&["--crawl", &s.url(), "--crawl-pages", "3", "--json"]);
        assert_eq!(code(&o), 0, "{}", text(&o));
        let v = stdout_json(&o);
        let step = v["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == "crawl")
            .unwrap_or_else(|| panic!("no crawl step: {v}"));
        assert_eq!(step["state"], "done", "{v}");
        assert!(s.crawl_dir().join("facts.jsonl").is_file());
        assert!(
            std::fs::read_to_string(s.crawl_dir().join("args.txt"))
                .unwrap()
                .contains("--max-pages 3")
        );
        // The same crawl, plain: the stop and the way to raise it.
        let o = s.init(&["--crawl", &s.url(), "--crawl-pages", "3"]);
        assert!(
            text(&o).contains(
                "stopped at the page limit; raise it with --crawl-pages / --crawl-minutes"
            ),
            "{}",
            text(&o)
        );
        // A second crawl rewrites the one study and the one memory.
        let studies = s.dir.read_dir_of("atlas/studies");
        assert_eq!(
            studies.iter().filter(|n| n.contains("site-crawl")).count(),
            1,
            "{studies:?}"
        );
        assert_eq!(crawl_memories(&s.dir), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_crawler_that_fails_does_not_fail_init() {
        // Exit 3 and no JSON: the step fails, init still finishes, nothing is kept.
        let s = site(Some(fake_agent(3, &[], false)), 1..=1).await;
        let o = s.init(&["--crawl", &s.url()]);
        let t = text(&o);
        assert_eq!(code(&o), 0, "{t}");
        assert!(t.contains("the crawler exited with 3"), "{t}");
        assert!(!s.crawl_dir().join("facts.jsonl").exists(), "{t}");
        assert!(!s.dir.join("atlas/studies").read_dir().is_ok_and(|mut d| {
            d.any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().contains("site-crawl")))
        }));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn without_the_agent_extension_it_says_so_in_a_line_and_reads_nothing() {
        let s = site(None, 0..=0).await;
        let o = s.init(&["--crawl", &s.url()]);
        let t = text(&o);
        assert_eq!(code(&o), 0, "{t}");
        assert!(t.contains("is not installed"), "{t}");
        assert!(t.contains("ext install inorbit/agent"), "{t}");
        assert!(!s.crawl_dir().exists(), "{t}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_dry_run_says_what_it_would_read_and_reads_nothing() {
        let s = site(Some(fake_agent(0, &[], true)), 0..=0).await;
        let o = s.init(&["--crawl", &s.url(), "--dry-run"]);
        let t = text(&o);
        assert_eq!(code(&o), 0, "{t}");
        assert!(t.contains("would: read"), "{t}");
        assert!(t.contains("nothing is sent to InOrbit"), "{t}");
        assert!(!s.crawl_dir().exists(), "{t}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_address_that_is_not_a_web_address_is_refused_before_anything_runs() {
        let s = site(Some(fake_agent(0, &[], true)), 0..=0).await;
        let o = s.init(&["--crawl", "ftp://acme.example"]);
        assert_eq!(code(&o), 2, "{}", text(&o));
        assert!(text(&o).contains("not a web address"), "{}", text(&o));
    }

    /// How many of the project's memory entries say a site was crawled (init seeds others).
    fn crawl_memories(dir: &Path) -> usize {
        std::fs::read_dir(dir.join(".inorbit/memory"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                std::fs::read_to_string(e.path())
                    .is_ok_and(|t| t.contains("was crawled on this machine"))
            })
            .count()
    }

    trait ReadDirOf {
        fn read_dir_of(&self, rel: &str) -> Vec<String>;
    }

    impl ReadDirOf for PathBuf {
        /// The file names in a folder under this one; none when it is not there.
        fn read_dir_of(&self, rel: &str) -> Vec<String> {
            std::fs::read_dir(self.join(rel))
                .map(|d| {
                    d.filter_map(Result::ok)
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default()
        }
    }
}
