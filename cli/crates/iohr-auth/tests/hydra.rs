//! Both sign-in grants, refresh and revocation against a real Ory Hydra, the identity
//! provider the platform runs. `mise run cli:hydra` starts one (the version the
//! platform pins) and sets `IOHR_TEST_HYDRA` and `IOHR_TEST_HYDRA_ADMIN`; without
//! them the test says so and returns, unless `IOHR_TEST_REQUIRE_HYDRA` is set.
//!
//! The test plays the browser: it follows Hydra's redirects with a cookie jar and
//! accepts the login, consent and device-code steps through Hydra's admin API, as the
//! platform's sign-in pages do.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stderr,
    reason = "tests fail by panicking; a skip is reported on stderr"
)]

use std::sync::Arc;

use iohr_auth::{
    AuthError, Authorization, Browser, Credential, Device, EntryKey, MemoryStore, Provider,
    Redacted, Session, Store,
};
use reqwest::header::LOCATION;
use url::Url;

const CLIENT: &str = "iohr-cli";

struct Hydra {
    public: String,
    admin: String,
}

fn hydra() -> Option<Hydra> {
    let public = std::env::var("IOHR_TEST_HYDRA")
        .ok()
        .filter(|v| !v.is_empty());
    let admin = std::env::var("IOHR_TEST_HYDRA_ADMIN")
        .ok()
        .filter(|v| !v.is_empty());
    if let (Some(public), Some(admin)) = (public, admin) {
        return Some(Hydra { public, admin });
    }
    let required = std::env::var("IOHR_TEST_REQUIRE_HYDRA").is_ok_and(|v| !v.is_empty());
    assert!(
        !required,
        "IOHR_TEST_HYDRA and IOHR_TEST_HYDRA_ADMIN are required"
    );
    eprintln!("hydra: skipped, no Hydra (run `mise run cli:hydra`)");
    None
}

/// The client exactly as the platform registers it (core devops/k8s/auth/seed-clients.sh).
async fn seed(h: &Hydra) {
    let http = reqwest::Client::new();
    let client = serde_json::json!({
        "client_id": CLIENT, "client_name": "iohr command line",
        "grant_types": ["authorization_code", "refresh_token", "urn:ietf:params:oauth:grant-type:device_code"],
        "response_types": ["code"],
        "scope": "openid offline_access iohr.api",
        "audience": ["iohr-api"],
        "token_endpoint_auth_method": "none",
        "redirect_uris": ["http://127.0.0.1/callback", "http://[::1]/callback"],
        "skip_consent": true, "skip_logout_consent": true,
        "access_token_strategy": "jwt"
    });
    let put = http
        .put(format!("{}/admin/clients/{CLIENT}", h.admin))
        .json(&client)
        .send()
        .await
        .unwrap();
    if put.status() == 404 {
        let post = http
            .post(format!("{}/admin/clients", h.admin))
            .json(&client)
            .send()
            .await
            .unwrap();
        // 409: a test running beside this one created it first.
        assert!(
            post.status().is_success() || post.status() == 409,
            "seeding the client: {}",
            post.status()
        );
    }
}

/// A browser: cookies kept, redirects not followed, so each hop can be inspected.
fn browser() -> reqwest::Client {
    reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

async fn location(b: &reqwest::Client, url: &str) -> String {
    let r = b.get(url).send().await.unwrap();
    assert!(r.status().is_redirection(), "{url} answered {}", r.status());
    r.headers()[LOCATION].to_str().unwrap().to_owned()
}

fn param(url: &str, name: &str) -> String {
    Url::parse(url)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == name)
        .unwrap_or_else(|| panic!("{name} in {url}"))
        .1
        .into_owned()
}

async fn accept(
    h: &Hydra,
    kind: &str,
    challenge_name: &str,
    challenge: &str,
    body: serde_json::Value,
) -> String {
    let r: serde_json::Value = reqwest::Client::new()
        .put(format!(
            "{}/admin/oauth2/auth/requests/{kind}/accept?{challenge_name}={challenge}",
            h.admin
        ))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    r["redirect_to"]
        .as_str()
        .unwrap_or_else(|| panic!("{kind} accept: {r}"))
        .to_owned()
}

/// Signs `subject` in through Hydra's login and consent, starting from a URL that
/// redirects to the login page; returns where Hydra sends the browser at the end.
async fn login_and_consent(h: &Hydra, b: &reqwest::Client, start: &str, subject: &str) -> String {
    let login = location(b, start).await;
    let next = accept(
        h,
        "login",
        "login_challenge",
        &param(&login, "login_challenge"),
        serde_json::json!({"subject": subject}),
    )
    .await;
    let consent = location(b, &next).await;
    let next = accept(
        h,
        "consent",
        "consent_challenge",
        &param(&consent, "consent_challenge"),
        serde_json::json!({"grant_scope": ["openid", "offline_access", "iohr.api"], "grant_access_token_audience": ["iohr-api"]}),
    )
    .await;
    location(b, &next).await
}

fn key() -> EntryKey {
    EntryKey::new("hydra".parse().unwrap(), "acc").unwrap()
}

/// Rewrites the stored session's expiry into the past, so the next call refreshes.
fn expire(store: &Arc<dyn Store>) -> String {
    let mut v: serde_json::Value =
        serde_json::from_str(store.get(&key()).unwrap().unwrap().expose()).unwrap();
    v["expires_at"] = serde_json::json!(0);
    let refresh = v["refresh_token"].as_str().unwrap().to_owned();
    store.set(&key(), &Redacted::new(v.to_string())).unwrap();
    refresh
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_sign_in_refresh_and_revocation() {
    let Some(h) = hydra() else { return };
    seed(&h).await;
    let provider = Provider::discover(&h.public, CLIENT).await.unwrap();
    let auth = Authorization::<Browser>::start(provider).await.unwrap();
    let url = auth.url().to_string();

    let b = browser();
    let (granted, ()) = tokio::join!(auth.finish(), async {
        let callback = login_and_consent(&h, &b, &url, "person-browser").await;
        assert!(callback.contains("/callback?code="), "{callback}");
        let page = b.get(&callback).send().await.unwrap().text().await.unwrap();
        assert!(page.contains("Signed in"));
    });
    let granted = granted.unwrap();
    assert_eq!(granted.subject(), "person-browser");

    let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let session = Session::create(granted, Arc::clone(&store), key())
        .await
        .unwrap();
    assert!(session.bearer().await.unwrap().expose().starts_with("eyJ"));

    // A real refresh rotates the refresh token and writes it back.
    let before = expire(&store);
    let session = Session::load(Arc::clone(&store), key(), &h.public, CLIENT)
        .await
        .unwrap();
    session.bearer().await.unwrap();
    let after: serde_json::Value =
        serde_json::from_str(store.get(&key()).unwrap().unwrap().expose()).unwrap();
    assert_ne!(after["refresh_token"].as_str().unwrap(), before, "rotated");

    // Revoked, the session cannot be refreshed any more.
    session.revoke().await.unwrap();
    expire(&store);
    let session = Session::load(Arc::clone(&store), key(), &h.public, CLIENT)
        .await
        .unwrap();
    assert!(matches!(
        session.bearer().await.unwrap_err(),
        AuthError::SessionEnded { .. }
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn device_sign_in() {
    let Some(h) = hydra() else { return };
    seed(&h).await;
    let provider = Provider::discover(&h.public, CLIENT).await.unwrap();
    let auth = Authorization::<Device>::start(provider).await.unwrap();
    let complete = auth
        .verification_uri_complete()
        .expect("a complete link")
        .to_string();
    let user_code = auth.user_code().to_owned();

    let b = browser();
    let (granted, ()) = tokio::join!(auth.finish(), async {
        // Hydra sends the browser to the platform's code page with a challenge.
        let page = location(&b, &complete).await;
        let challenge = param(&page, "device_challenge");
        let next = accept(
            &h,
            "device",
            "device_challenge",
            &challenge,
            serde_json::json!({"user_code": user_code}),
        )
        .await;
        let done = login_and_consent(&h, &b, &next, "person-device").await;
        assert!(done.contains("/device/done"), "{done}");
    });
    let granted = granted.unwrap();
    assert_eq!(granted.subject(), "person-device");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_device_code_is_not_accepted() {
    let Some(h) = hydra() else { return };
    seed(&h).await;
    let provider = Provider::discover(&h.public, CLIENT).await.unwrap();
    let auth = Authorization::<Device>::start(provider).await.unwrap();
    let b = browser();
    let page = location(&b, auth.verification_uri_complete().unwrap().as_str()).await;
    let r = reqwest::Client::new()
        .put(format!(
            "{}/admin/oauth2/auth/requests/device/accept?device_challenge={}",
            h.admin,
            param(&page, "device_challenge")
        ))
        .json(&serde_json::json!({"user_code": "WRONGCOD"}))
        .send()
        .await
        .unwrap();
    assert!(
        !r.status().is_success(),
        "a wrong code is refused: {}",
        r.status()
    );
}
