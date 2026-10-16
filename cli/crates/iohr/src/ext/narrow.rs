//! A token narrowed to an extension's declared scopes, for a signed-in person (ADR 0012,
//! point 6).
//!
//! A person's own access token carries only the umbrella scope `iohr.api`, so it never
//! "holds" an extension's `identity:read`, and handing it over would give the extension
//! everything the person can do. The sign-in service cannot narrow a token yet (no
//! RFC 8693 token exchange). What the platform can do today is a key: `iohr` creates an
//! account key with exactly the requested scopes, exchanges it for a 15-minute token
//! (the same exchange any program with a key makes), and revokes the key at once. The
//! key's secret lives in memory for that exchange and is never printed or stored; the
//! extension receives the token and nothing else.

use std::time::Duration;

use inorbithr::{ClientCredentials, Method, Secret, TokenProvider as _};
use iohr_auth::{Claims, Redacted};
use serde::Deserialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::socket::{MintError, Minted};
use crate::context::{Api, note};

/// The deadline on the key's token exchange. It is shorter than the channel's mint
/// deadline (`MINT_TIMEOUT`, 30 s), so a token endpoint that never answers is cut here
/// and the key is revoked on the normal path, never by a dropped future.
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(15);

/// The deadline on a revoke started by a dropped call (the connection task was cancelled
/// or the mint deadline fired): short, because nothing waits for it.
const REVOKE_TIMEOUT: Duration = Duration::from_secs(10);

/// What the key create answers; every field optional so an answer with a key id but
/// without a secret can still be revoked.
#[derive(Deserialize, Default)]
struct Made {
    #[serde(default)]
    key: MadeKey,
    #[serde(default)]
    secret: Option<Secret<String>>,
    #[serde(default)]
    token_url: Option<String>,
}

#[derive(Deserialize, Default)]
struct MadeKey {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
}

/// Revokes a key when the call that made it ends in any way. Normal exits revoke inline
/// and disarm it; if the future is dropped first (cancel, mint deadline) `drop` spawns
/// a detached revoke with its own deadline, so a cancel never leaves the key alive.
struct Revoke {
    api: Api,
    path: String,
    id: String,
    armed: bool,
}

impl Revoke {
    async fn run(&mut self) {
        let gone = self.api.send(Method::Delete, &self.path, &[], None).await;
        self.armed = false;
        if let Err(e) = gone {
            note(&format!(
                "could not revoke the short-lived key {} ({e}); `iohr token revoke {}` does",
                self.id, self.id
            ));
        }
    }
}

impl Drop for Revoke {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            note(&format!(
                "the short-lived key {} was not revoked (no runtime); `iohr token revoke {}` does",
                self.id, self.id
            ));
            return;
        };
        let mut late = Revoke {
            api: self.api.handle(),
            path: std::mem::take(&mut self.path),
            id: std::mem::take(&mut self.id),
            armed: false,
        };
        rt.spawn(async move {
            if tokio::time::timeout(REVOKE_TIMEOUT, late.run())
                .await
                .is_err()
            {
                note(&format!(
                    "could not revoke the short-lived key {} in time; `iohr token revoke {}` does",
                    late.id, late.id
                ));
            }
        });
    }
}

/// A token for `scopes` (already checked against the manifest) and nothing more.
///
/// Once the key has an id it is revoked on every path: success, failure, the exchange
/// deadline, and a dropped future (a detached revoke). Only a cancel during the create
/// call itself, before any id is known, cannot be revoked from here.
pub(crate) async fn narrowed(
    api: &Api,
    account: &str,
    scopes: &[String],
) -> Result<Minted, MintError> {
    narrowed_within(api, account, scopes, EXCHANGE_TIMEOUT).await
}

async fn narrowed_within(
    api: &Api,
    account: &str,
    scopes: &[String],
    exchange_deadline: Duration,
) -> Result<Minted, MintError> {
    let keys = format!("/v1/accounts/orgs/{account}/keys");
    let body = serde_json::json!({
        "name": format!("iohr-ext-{}", OffsetDateTime::now_utc().unix_timestamp()),
        "scopes": scopes,
    });
    let made: Made = api
        .send(Method::Post, &keys, &[], Some(&body))
        .await
        .and_then(|r| r.json())
        .map_err(|e| match e.status() {
            Some(401) => MintError::Unavailable(format!(
                "your session is not valid: sign in again (`iohr login`) to get a token \
                 narrowed to {}: {e}",
                scopes.join(" ")
            )),
            Some(403) => MintError::Forbidden(format!(
                "a token narrowed to {} needs an admin or owner of account {account} \
                 (it creates a short-lived key): {e}",
                scopes.join(" ")
            )),
            _ => MintError::Unavailable(format!("could not create a short-lived key: {e}")),
        })?;
    let Made {
        key: MadeKey { id, client_id },
        secret,
        token_url,
    } = made;
    // From here the key exists: whatever happens next, it is revoked, also if this
    // future is dropped.
    let mut guard = id.map(|id| Revoke {
        api: api.handle(),
        path: format!("{keys}/{id}"),
        id,
        armed: true,
    });
    let outcome = match tokio::time::timeout(
        exchange_deadline,
        exchange(client_id, secret, token_url, scopes),
    )
    .await
    {
        Ok(r) => r,
        Err(_) => Err(MintError::Unavailable(format!(
            "the short-lived key's token endpoint did not answer in {} s; the key is revoked",
            exchange_deadline.as_secs().max(1)
        ))),
    };
    if let Some(g) = guard.as_mut() {
        g.run().await;
    } else if outcome.is_ok() {
        note("the key answer named no id, so it was not revoked");
    }
    outcome
}

async fn exchange(
    client_id: Option<String>,
    secret: Option<Secret<String>>,
    url: Option<String>,
    scopes: &[String],
) -> Result<Minted, MintError> {
    let unavailable = |m: &str| MintError::Unavailable(m.to_owned());
    let (Some(client_id), Some(secret), Some(url)) = (
        client_id.filter(|s| !s.is_empty()),
        secret.filter(|s| !s.expose().is_empty()),
        url.filter(|s| !s.is_empty()),
    ) else {
        return Err(unavailable(
            "the key answer lacks a client id, secret or token url",
        ));
    };
    let url = url::Url::parse(&url).map_err(|_| unavailable("the key's token url is not a URL"))?;
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback(&url))) {
        return Err(unavailable(
            "the key's token url must be https (plain http only to this machine)",
        ));
    }
    let token = ClientCredentials::new(client_id, secret, scopes.iter().cloned())
        .map_err(|e| MintError::Unavailable(e.to_string()))?
        .token_url(url)
        .token()
        .await
        .map_err(|e| MintError::Unavailable(format!("the short-lived key's token: {e}")))?;
    let expires_at = Claims::read(token.expose(), OffsetDateTime::now_utc())
        .ok()
        .and_then(|c| c.expires_at())
        .or_else(|| {
            token.expires_at().map(|at| {
                OffsetDateTime::now_utc() + at.saturating_duration_since(std::time::Instant::now())
            })
        })
        .and_then(|t| t.format(&Rfc3339).ok())
        .unwrap_or_default();
    Ok(Minted {
        access_token: Redacted::new(token.expose().to_owned()),
        expires_at,
    })
}

fn loopback(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(d)) => d == "localhost",
        None => false,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use std::sync::Arc;

    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use iohr_auth::{Redacted, StaticToken};
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use std::time::Duration;

    use super::{narrowed, narrowed_within};
    use crate::context::{AnyCredential, Api};
    use crate::ext::socket::MintError;

    fn jwt(scopes: &[&str]) -> String {
        let enc = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
        format!(
            "{}.{}.SIGNATUREMARKER",
            enc(serde_json::json!({"alg": "RS256"})),
            enc(serde_json::json!({
                "sub": "ak_k1", "aud": ["iohr-api"], "exp": 4_102_444_800_i64, "scp": scopes
            }))
        )
    }

    fn api(server: &MockServer) -> Api {
        let person = Arc::new(AnyCredential::Token(StaticToken::new(Redacted::new(jwt(
            &["iohr.api"],
        )))));
        Api::new(&server.uri(), person, false).unwrap()
    }

    fn key_answer(server: &MockServer) -> serde_json::Value {
        serde_json::json!({
            "key": {"id": "key_1", "client_id": "ak_1"},
            "secret": "KEYSECRETMARKER",
            "token_url": format!("{}/oauth2/token", server.uri()),
        })
    }

    /// A person's token holds only `iohr.api`: the extension still gets a token with
    /// exactly the scopes it declared, from a key that is revoked once it is used.
    #[tokio::test]
    async fn a_person_gets_a_token_narrowed_to_the_scopes_and_the_key_is_revoked() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/accounts/orgs/acc1/keys"))
            .and(body_string_contains(r#""scopes":["identity:read"]"#))
            .respond_with(ResponseTemplate::new(200).set_body_json(key_answer(&server)))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .and(body_string_contains("scope=identity%3Aread"))
            .and(body_string_contains("grant_type=client_credentials"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": jwt(&["identity:read"]),
                "token_type": "bearer",
                "expires_in": 900,
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/v1/accounts/orgs/acc1/keys/key_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        let m = narrowed(&api(&server), "acc1", &["identity:read".to_owned()])
            .await
            .unwrap();
        assert_eq!(m.access_token.expose(), &jwt(&["identity:read"]));
        assert_eq!(m.expires_at, "2100-01-01T00:00:00Z");
        // The mocks' `expect` checks run when the server drops: the key was created
        // once, exchanged once and revoked once.
    }

    /// A failed exchange still revokes the key, and the error never carries its secret.
    #[tokio::test]
    async fn a_failed_exchange_still_revokes_the_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/accounts/orgs/acc1/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(key_answer(&server)))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(serde_json::json!({"error": "invalid_client"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/v1/accounts/orgs/acc1/keys/key_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        let e = narrowed(&api(&server), "acc1", &["identity:read".to_owned()])
            .await
            .unwrap_err();
        let MintError::Unavailable(m) = e else {
            panic!("{e:?}")
        };
        assert!(!m.contains("KEYSECRETMARKER"), "{m}");
    }

    /// Someone who may not create keys is refused with the reason, and nothing is minted.
    #[tokio::test]
    async fn a_member_who_cannot_create_keys_is_refused_with_the_reason() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/accounts/orgs/acc1/keys"))
            .respond_with(ResponseTemplate::new(403).set_body_json(
                serde_json::json!({"error": {"code": "permission_denied", "message": "admins only"}}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let e = narrowed(&api(&server), "acc1", &["identity:read".to_owned()])
            .await
            .unwrap_err();
        let MintError::Forbidden(m) = e else {
            panic!("{e:?}")
        };
        assert!(m.contains("admin or owner of account acc1"), "{m}");
    }

    async fn deletes(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method.as_str() == "DELETE")
            .count()
    }

    /// A key and a token endpoint that holds the answer for an hour.
    async fn hanging_token_endpoint() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/accounts/orgs/acc1/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(key_answer(&server)))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3600)))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/v1/accounts/orgs/acc1/keys/key_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;
        server
    }

    /// A token endpoint that never answers: the exchange deadline fires inside
    /// `narrowed`, the key is revoked, and the error says so. Real (short) deadlines,
    /// not a paused clock: the mock server answers over real sockets, where a paused
    /// clock would also expire the revoke call's own timers while it waits on I/O.
    #[tokio::test]
    async fn an_exchange_that_never_answers_is_cut_and_the_key_revoked() {
        let server = hanging_token_endpoint().await;
        let e = narrowed_within(
            &api(&server),
            "acc1",
            &["identity:read".to_owned()],
            Duration::from_millis(300),
        )
        .await
        .unwrap_err();
        let MintError::Unavailable(m) = e else {
            panic!("{e:?}")
        };
        assert!(m.contains("did not answer"), "{m}");
        assert!(!m.contains("KEYSECRETMARKER"), "{m}");
        assert_eq!(deletes(&server).await, 1);
    }

    /// The outer future is dropped while the exchange hangs (the channel's mint deadline,
    /// a cancelled connection task): the guard still revokes the key, detached.
    #[tokio::test]
    async fn a_dropped_call_still_revokes_the_key() {
        let server = hanging_token_endpoint().await;
        let cut = tokio::time::timeout(
            Duration::from_millis(400),
            narrowed_within(
                &api(&server),
                "acc1",
                &["identity:read".to_owned()],
                Duration::from_secs(3600),
            ),
        )
        .await;
        assert!(
            cut.is_err(),
            "the call should have been cut by the outer deadline"
        );
        for _ in 0..100 {
            if deletes(&server).await == 1 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the key was not revoked after the call was dropped");
    }

    /// An expired session is told to sign in again, not that it needs an admin.
    #[tokio::test]
    async fn an_expired_session_is_told_to_sign_in_again() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/accounts/orgs/acc1/keys"))
            .respond_with(ResponseTemplate::new(401).set_body_json(
                serde_json::json!({"error": {"code": "unauthenticated", "message": "expired"}}),
            ))
            .mount(&server)
            .await;
        let e = narrowed(&api(&server), "acc1", &["identity:read".to_owned()])
            .await
            .unwrap_err();
        let MintError::Unavailable(m) = e else {
            panic!("{e:?}")
        };
        assert!(m.contains("sign in again"), "{m}");
        assert!(!m.contains("admin or owner"), "{m}");
    }
}
