//! No secret is ever printed (RFC 0231, SR-10): a marker in every token, the device
//! code and the authorization code, then every place a program could print one. The
//! tracing output, `Debug` of every public type of the flows, and the text of every
//! error. A secret in any of them fails the test with the place it showed up in.
//!
//! Also the size cap on the sign-in service's answers (study 0094.60).

#![cfg(feature = "login")]
#![allow(clippy::unwrap_used, reason = "tests fail by panicking")]

use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use inorbithr::Secret;
use inorbithr::login::{
    EntryKey, Login, LoginError, MemoryStore, Provider, Session, SignOut, Store,
};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CLIENT: &str = "iohr-cli";
const ACCESS_MARK: &str = "ACCESSMARKER9f3a";
const REFRESH_MARK: &str = "REFRESHMARKER7c1d";
const DEVICE_MARK: &str = "DEVICECODEMARKER42e0";
const CODE_MARK: &str = "AUTHCODEMARKER5b6e";

fn jwt(claims: &serde_json::Value) -> String {
    let enc = |v: &serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.c2ln",
        enc(&serde_json::json!({"alg": "RS256"})),
        enc(claims)
    )
}

fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

fn access() -> String {
    jwt(&serde_json::json!({
        "sub": "person-1", "aud": ["iohr-api"], "exp": now() + 900,
        "scp": ["openid", "offline_access", "iohr.api"], "org": "acc_1", "plan": "free",
        "marker": ACCESS_MARK
    }))
}

fn tokens(issuer: &str) -> serde_json::Value {
    serde_json::json!({
        "access_token": access(), "token_type": "bearer", "expires_in": 900,
        "refresh_token": REFRESH_MARK,
        "id_token": jwt(&serde_json::json!({"iss": issuer, "aud": [CLIENT], "sub": "person-1"})),
    })
}

fn key() -> EntryKey {
    EntryKey::new("secrets".parse().unwrap(), "acc_1").unwrap()
}

/// Every secret the flows hold.
fn secrets() -> [String; 5] {
    [
        ACCESS_MARK.into(),
        REFRESH_MARK.into(),
        DEVICE_MARK.into(),
        CODE_MARK.into(),
        access(),
    ]
}

/// Collects what a program's tracing subscriber would see: every span and event field.
#[derive(Clone, Default)]
struct Tape(Arc<Mutex<String>>);

impl Tape {
    fn text(&self) -> String {
        self.0.lock().unwrap().clone()
    }
}

#[cfg(feature = "tracing")]
mod tape {
    use super::Tape;
    use std::fmt::Write as _;
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Metadata, Subscriber};

    struct Write<'a>(&'a mut String);

    impl Visit for Write<'_> {
        fn record_debug(&mut self, f: &Field, v: &dyn std::fmt::Debug) {
            let _ = write!(self.0, " {}={v:?}", f.name());
        }
    }

    impl Subscriber for Tape {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, a: &Attributes<'_>) -> Id {
            let mut s = self.0.lock().unwrap();
            let _ = write!(s, "\nspan {}", a.metadata().name());
            a.record(&mut Write(&mut s));
            Id::from_u64(1)
        }
        fn record(&self, _: &Id, v: &Record<'_>) {
            v.record(&mut Write(&mut self.0.lock().unwrap()));
        }
        fn record_follows_from(&self, _: &Id, _: &Id) {}
        fn event(&self, e: &Event<'_>) {
            let mut s = self.0.lock().unwrap();
            let _ = write!(s, "\nevent {}", e.metadata().target());
            e.record(&mut Write(&mut s));
        }
        fn enter(&self, _: &Id) {}
        fn exit(&self, _: &Id) {}
    }
}

fn assert_clean(what: &str, shown: &str) {
    for s in secrets() {
        assert!(!shown.contains(&s), "{what} printed a secret: {shown}");
    }
}

async fn server() -> MockServer {
    let server = MockServer::start().await;
    let uri = server.uri();
    Mock::given(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": uri,
            "authorization_endpoint": format!("{uri}/oauth2/auth"),
            "token_endpoint": format!("{uri}/oauth2/token"),
            "device_authorization_endpoint": format!("{uri}/oauth2/device/auth"),
            "revocation_endpoint": format!("{uri}/oauth2/revoke"),
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth2/device/auth"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "device_code": DEVICE_MARK, "user_code": "ABCD2345",
            "verification_uri": format!("{uri}/device"),
            "verification_uri_complete": format!("{uri}/device?user_code=ABCD2345"),
            "expires_in": 600, "interval": 1,
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .and(body_string_contains("grant_type=urn"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tokens(&uri)))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tokens(&uri)))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth2/revoke"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    server
}

/// A sign-in service that answers with a secret in its error text: the error strings
/// of the SDK must not repeat it either.
async fn leaky_server() -> MockServer {
    let server = MockServer::start().await;
    let uri = server.uri();
    Mock::given(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": uri,
            "authorization_endpoint": format!("{uri}/oauth2/auth"),
            "token_endpoint": format!("{uri}/oauth2/token"),
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .respond_with(
            ResponseTemplate::new(500).set_body_string(format!("{REFRESH_MARK} {DEVICE_MARK}")),
        )
        .mount(&server)
        .await;
    server
}

#[allow(clippy::too_many_lines, reason = "one scenario, read top to bottom")]
#[tokio::test]
async fn no_secret_reaches_tracing_debug_or_an_error_string() {
    // The browser is played by a plain GET: a test never opens one (study 0150).
    let tape = Tape::default();
    #[cfg(feature = "tracing")]
    let _guard = tracing::subscriber::set_default(tape.clone());

    let server = server().await;
    let mut shown = vec![];

    // The browser flow, the browser played by a plain GET.
    let provider = Provider::discover(&server.uri(), CLIENT).await.unwrap();
    shown.push(("Debug of Provider", format!("{provider:?}")));
    let auth = Login::browser(&provider).await.unwrap();
    shown.push(("Debug of Authorization<Browser>", format!("{auth:?}")));
    let q: std::collections::HashMap<_, _> = auth.url().query_pairs().into_owned().collect();
    let redirect = q["redirect_uri"].clone();
    let state = q["state"].clone();
    let page = tokio::spawn(async move {
        reqwest::get(format!("{redirect}?code={CODE_MARK}&state={state}"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
    });
    let granted = auth.finish().await.unwrap();
    shown.push(("the browser's page", page.await.unwrap()));
    shown.push(("Debug of Authorization<Granted>", format!("{granted:?}")));

    // The device flow: the user code and link are shown, never the device code.
    let dev = Login::device(&provider).await.unwrap();
    shown.push(("Debug of Authorization<Device>", format!("{dev:?}")));
    shown.push(("user code", dev.user_code().to_owned()));
    shown.push(("verification uri", dev.verification_uri().to_string()));
    let granted_device = dev.finish().await.unwrap();

    // Saved, loaded, used, signed out.
    let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let session = Session::save(granted, Arc::clone(&store), key())
        .await
        .unwrap();
    shown.push(("Debug of Session", format!("{session:?}")));
    let token = session.access_token().await.unwrap();
    shown.push(("Debug of the access token", format!("{token:?} {token}")));
    let other = Session::save(granted_device, Arc::new(MemoryStore::default()), key())
        .await
        .unwrap();
    let out = other.sign_out().await.unwrap();
    assert_eq!(out, SignOut::Revoked);
    shown.push(("Debug of SignOut", format!("{out:?}")));
    shown.push((
        "Debug of Secret",
        format!("{:?}", Secret::new(REFRESH_MARK.to_owned())),
    ));

    // Every error a leaky service can cause.
    let leaky = leaky_server().await;
    let lp = Provider::discover(&leaky.uri(), CLIENT).await.unwrap();
    let mut errors: Vec<LoginError> = vec![];
    errors.push(Login::device(&lp).await.unwrap_err());
    let b = Login::browser(&lp).await.unwrap();
    let q: std::collections::HashMap<_, _> = b.url().query_pairs().into_owned().collect();
    let (redirect, state) = (q["redirect_uri"].clone(), q["state"].clone());
    let bad_state = tokio::spawn({
        let redirect = redirect.clone();
        async move {
            reqwest::get(format!("{redirect}?code={CODE_MARK}&state=wrong"))
                .await
                .unwrap()
                .text()
                .await
                .unwrap()
        }
    });
    errors.push(b.finish().await.unwrap_err());
    shown.push(("the failure page", bad_state.await.unwrap()));
    let b = Login::browser(&lp).await.unwrap();
    let q: std::collections::HashMap<_, _> = b.url().query_pairs().into_owned().collect();
    let (redirect, state2) = (q["redirect_uri"].clone(), q["state"].clone());
    let _ = state;
    let exchange = tokio::spawn(async move {
        reqwest::get(format!("{redirect}?code={CODE_MARK}&state={state2}"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
    });
    errors.push(b.finish().await.unwrap_err());
    shown.push(("the exchange failure page", exchange.await.unwrap()));
    errors.push(
        Provider::discover("http://auth.example.com", CLIENT)
            .await
            .unwrap_err(),
    );
    errors.push(LoginError::NotSignedIn {
        profile: "p".into(),
    });
    errors.push(LoginError::SessionEnded {
        profile: "p".into(),
    });
    for e in &errors {
        shown.push(("an error's Display", e.to_string()));
        shown.push(("an error's Debug", format!("{e:?}")));
    }

    for (what, text) in &shown {
        assert_clean(what, text);
    }
    assert_clean("the tracing output", &tape.text());
    assert!(
        errors.len() >= 5 && shown.len() >= 15,
        "the test looked in enough places: {}",
        shown.len()
    );
}

#[tokio::test]
async fn an_oversized_answer_from_the_sign_in_service_is_refused_unread() {
    let server = MockServer::start().await;
    Mock::given(path("/.well-known/openid-configuration"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(vec![b' '; 300 * 1024], "application/json"),
        )
        .mount(&server)
        .await;
    let e = Provider::discover(&server.uri(), CLIENT).await.unwrap_err();
    assert!(e.to_string().contains("too much"), "{e}");
}
