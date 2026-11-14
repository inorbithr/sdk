//! The `login` source of the default credential chain (RFC 0231, `docs/config.md`
//! section 5.1 and 5.6): a profile with `kind = "person"` and a stored session is
//! signed in by the SDK itself, with no `iohr` on the machine.
//!
//! Every test reads the session from an in-memory store injected through
//! `LoadOptions::login_store`, so none touches a keychain or the real home folder.

#![cfg(feature = "login")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use inorbithr::config::LoadOptions;
use inorbithr::login::{EntryKey, MemoryStore, ProfileName, Store, StoredSession};
use inorbithr::public::Surface as _;
use inorbithr::{Client, Public, Secret, TokenProvider};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Stands in for the access and refresh tokens: it must never show up in a string the
/// SDK prints.
const ACCESS: &str = "MARKER-ACCESS-7f3a91";
const REFRESH: &str = "MARKER-REFRESH-b20c44";

fn key() -> EntryKey {
    EntryKey::new("dev".parse::<ProfileName>().unwrap(), "acc_1").unwrap()
}

fn entry(access: &str, refresh: &str, expires_in_s: i64) -> Secret<String> {
    let at = time::OffsetDateTime::now_utc().unix_timestamp() + expires_in_s;
    Secret::new(
        json!({ "refresh_token": refresh, "access_token": access, "expires_at": at }).to_string(),
    )
}

fn signed_in(access: &str, refresh: &str, expires_in_s: i64) -> Arc<MemoryStore> {
    let store = Arc::new(MemoryStore::default());
    store
        .set(&key(), &entry(access, refresh, expires_in_s))
        .unwrap();
    store
}

/// A folder with a `config.toml` holding a person profile `dev` for `issuer`, and the
/// environment of a program that sets nothing but the API address. `PATH` is empty:
/// there is no `iohr` to find.
struct Machine {
    dir: tempfile::TempDir,
    env: BTreeMap<String, String>,
}

impl Machine {
    fn new(api: &str, issuer: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            format!(
                "default = \"dev\"\n[profiles.dev]\nkind = \"person\"\naccount = \"acc_1\"\n\
                 storage = \"keyring\"\nissuer = \"{issuer}\"\nclient_id = \"iohr-cli\"\n"
            ),
        )
        .unwrap();
        let env = BTreeMap::from([
            ("INORBIT_BASE_URL".to_owned(), api.to_owned()),
            ("PATH".to_owned(), String::new()),
        ]);
        Self { dir, env }
    }

    fn file(&self) -> PathBuf {
        self.dir.path().join("config.toml")
    }

    fn options(&self, store: Arc<dyn Store>) -> LoadOptions {
        let home = self.dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        LoadOptions::new()
            .env(self.env.clone())
            .home(Some(home))
            .cwd(self.dir.path())
            .login_store(store)
    }

    fn client(&self, store: Arc<dyn Store>) -> Result<Client<Public>, inorbithr::Error> {
        Client::<Public>::builder()
            .config_file(self.file())
            .load_options(self.options(store))
            .load()
    }
}

async fn api_expecting(bearer: &str) -> MockServer {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .and(header("authorization", format!("Bearer {bearer}").as_str()))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({ "subject": "usr_1", "kind": "user", "scopes": ["iohr.api"] }),
            ),
        )
        .expect(1..)
        .mount(&api)
        .await;
    api
}

fn credential_source(client: &Client<Public>) -> Value {
    client.config().describe()["credential"]["source"].clone()
}

#[tokio::test]
async fn a_signed_in_profile_works_with_no_iohr_on_the_path() {
    let api = api_expecting(ACCESS).await;
    let machine = Machine::new(&api.uri(), &api.uri());
    let client = machine
        .client(signed_in(ACCESS, REFRESH, 900))
        .expect("the client loads from the stored session");

    let me = client
        .me()
        .await
        .expect("the call carries the stored token");

    assert_eq!(me.raw.status, 200);
    assert_eq!(credential_source(&client), "login");
}

#[cfg(unix)]
#[tokio::test]
async fn the_chain_picks_login_over_cli_when_both_apply() {
    use std::os::unix::fs::PermissionsExt as _;

    let api = api_expecting(ACCESS).await;
    let machine = Machine::new(&api.uri(), &api.uri());
    // An `iohr` that would answer, and records that it was run.
    let ran = machine.dir.path().join("iohr-ran");
    let script = machine.dir.path().join("iohr");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ntouch '{}'\nprintf '{{\"access_token\":\"from-the-cli\"}}'\n",
            ran.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let client = Client::<Public>::builder()
        .config_file(machine.file())
        .cli_path(&script)
        .load_options(machine.options(signed_in(ACCESS, REFRESH, 900)))
        .load()
        .expect("the client loads");
    client.me().await.expect("the stored token is sent");

    assert_eq!(credential_source(&client), "login");
    assert!(
        !ran.exists(),
        "the cli source must not run when login applies"
    );
}

#[tokio::test]
async fn login_is_skipped_with_a_reason_when_the_store_has_no_entry() {
    let api = MockServer::start().await;
    let machine = Machine::new(&api.uri(), &api.uri());

    let err = machine
        .client(Arc::new(MemoryStore::default()))
        .expect_err("no credential applies");

    let text = err.to_string();
    assert!(
        text.contains("no stored session for profile dev"),
        "the reason is named: {text}"
    );
}

#[tokio::test]
async fn login_is_only_for_a_person_profile() {
    let api = MockServer::start().await;
    let machine = Machine::new(&api.uri(), &api.uri());
    let text = std::fs::read_to_string(machine.file())
        .unwrap()
        .replace("kind = \"person\"", "kind = \"token\"");
    std::fs::write(machine.file(), text).unwrap();

    let err = machine
        .client(signed_in(ACCESS, REFRESH, 900))
        .expect_err("a token profile has no sign-in");

    assert!(
        err.to_string().contains("is not a person's sign-in"),
        "{err}"
    );
}

#[tokio::test]
async fn credential_sources_can_leave_login_out() {
    let api = MockServer::start().await;
    let machine = Machine::new(&api.uri(), &api.uri());

    let err = Client::<Public>::builder()
        .config_file(machine.file())
        .credential_sources(["env", "file"])
        .load_options(machine.options(signed_in(ACCESS, REFRESH, 900)))
        .load()
        .expect_err("login is not allowed");

    assert!(
        err.to_string().contains("not in credential_sources"),
        "{err}"
    );
}

#[tokio::test]
async fn no_token_reaches_a_debug_an_error_or_a_description() {
    // The API refuses the token, so there is an error string to inspect.
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": { "code": "unauthenticated", "message": "no" }
        })))
        .mount(&api)
        .await;
    let machine = Machine::new(&api.uri(), &api.uri());
    let store = signed_in(ACCESS, REFRESH, 900);
    let client = machine.client(store.clone()).expect("the client loads");

    let err = client.me().await.expect_err("a 401");
    let session = StoredSession::open(store, key(), &api.uri(), "iohr-cli");
    let token = session.token().await.expect("a token");

    let shown = [
        format!("{client:?}"),
        format!("{session:?}"),
        format!("{token:?}"),
        format!("{err}"),
        format!("{err:?}"),
        client.config().describe().to_string(),
        machine
            .client(Arc::new(MemoryStore::default()))
            .err()
            .map(|e| format!("{e} {e:?}"))
            .unwrap_or_default(),
    ];
    for text in shown {
        assert!(!text.contains(ACCESS), "the access token is in: {text}");
        assert!(!text.contains(REFRESH), "the refresh token is in: {text}");
    }
    assert_eq!(token.expose(), ACCESS, "the provider hands it out");
}

/// A JWT the sign-in checks accept: the API's audience, unsigned.
fn jwt(id: &str) -> String {
    use base64::Engine as _;
    let enc = |v: &Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.c2ln",
        enc(&json!({ "alg": "RS256" })),
        enc(&json!({ "sub": "usr_1", "aud": ["iohr-api"], "jti": id }))
    )
}

#[tokio::test]
async fn invalidate_makes_the_next_call_get_a_fresh_token() {
    let issuer = MockServer::start().await;
    let base = issuer.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": base,
            "authorization_endpoint": format!("{base}/oauth2/auth"),
            "token_endpoint": format!("{base}/oauth2/token"),
            "device_authorization_endpoint": format!("{base}/oauth2/device/auth"),
            "revocation_endpoint": format!("{base}/oauth2/revoke"),
        })))
        .mount(&issuer)
        .await;
    let renewed = jwt("renewed");
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": renewed, "refresh_token": "rt-2",
            "token_type": "bearer", "expires_in": 900
        })))
        .expect(1)
        .mount(&issuer)
        .await;
    let store = signed_in(ACCESS, REFRESH, 900);
    let session = StoredSession::open(store.clone(), key(), &base, "iohr-cli");

    let first = session.token().await.expect("the stored token");
    let again = session.token().await.expect("cached");
    assert_eq!(first.expose(), ACCESS);
    assert_eq!(again.expose(), ACCESS, "a fresh token is cached");

    session.invalidate().await;
    let next = session.token().await.expect("a refreshed token");

    assert_eq!(next.expose(), renewed);
    let stored = store.get(&key()).unwrap().expect("the entry");
    assert!(
        stored.expose().contains("rt-2"),
        "the rotated refresh token is written back"
    );
}

#[tokio::test]
async fn a_failed_refresh_is_logged_like_any_credential_and_names_no_token() {
    use std::sync::Mutex;

    let issuer = MockServer::start().await;
    let base = issuer.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": base,
            "authorization_endpoint": format!("{base}/oauth2/auth"),
            "token_endpoint": format!("{base}/oauth2/token"),
        })))
        .mount(&issuer)
        .await;
    // The first refresh hands out a token that lives two seconds; the next is refused.
    let short = jwt("short");
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": short, "refresh_token": "rt-2",
            "token_type": "bearer", "expires_in": 2
        })))
        .up_to_n_times(1)
        .mount(&issuer)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({ "error": "invalid_grant" })))
        .mount(&issuer)
        .await;
    let api = api_expecting(&short).await;
    let machine = Machine::new(&api.uri(), &base);
    // 30 s left: the stored token is not usable, so the first call refreshes.
    let store = signed_in(ACCESS, REFRESH, 30);

    let records = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = Arc::clone(&records);
    let client = Client::<Public>::builder()
        .config_file(machine.file())
        .log(inorbithr::LogLevel::Warn)
        .logger(move |r| sink.lock().unwrap().push(format!("{r:?}")))
        .load_options(machine.options(store))
        .load()
        .expect("the client loads");
    client.me().await.expect("the refreshed token works");
    // Past the point where the cache wants a refresh, still before the token expires.
    tokio::time::sleep(std::time::Duration::from_millis(1700)).await;
    client.me().await.expect("the still-valid token is kept");

    let seen = records.lock().unwrap().join("\n");
    assert!(seen.contains("token_refresh_failed"), "no record: {seen}");
    for token in [ACCESS, REFRESH, "rt-2", short.as_str()] {
        assert!(!seen.contains(token), "a token is in a log record: {seen}");
    }
}
