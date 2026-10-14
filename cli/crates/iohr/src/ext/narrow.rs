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

use inorbithr::{ClientCredentials, Method, Secret, TokenProvider as _};
use iohr_auth::{Claims, Redacted};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::socket::{MintError, Minted};
use crate::context::{Api, note};

/// A token for `scopes` (already checked against the manifest) and nothing more.
pub(crate) async fn narrowed(
    api: &Api,
    account: &str,
    scopes: &[String],
) -> Result<Minted, MintError> {
    let keys = format!("/v1/accounts/orgs/{account}/keys");
    let body = serde_json::json!({
        "name": format!("iohr-ext-{}", OffsetDateTime::now_utc().unix_timestamp()),
        "scopes": scopes,
    });
    let made: Value = api
        .send(Method::Post, &keys, &[], Some(&body))
        .await
        .and_then(|r| r.json())
        .map_err(|e| match e.status() {
            Some(401 | 403) => MintError::Forbidden(format!(
                "a token narrowed to {} needs an admin or owner of account {account} \
                 (it creates a short-lived key): {e}",
                scopes.join(" ")
            )),
            _ => MintError::Unavailable(format!("could not create a short-lived key: {e}")),
        })?;
    let key_id = text(&made, &["key", "id"]);
    // From here the key exists: whatever happens next, it is revoked.
    let outcome = exchange(&made, scopes).await;
    if let Some(id) = key_id {
        let gone = api
            .send(Method::Delete, &format!("{keys}/{id}"), &[], None)
            .await;
        if let Err(e) = gone {
            note(&format!(
                "could not revoke the short-lived key {id} ({e}); `iohr token revoke {id}` does"
            ));
        }
    } else if outcome.is_ok() {
        note("the key answer named no id, so it was not revoked");
    }
    outcome
}

async fn exchange(made: &Value, scopes: &[String]) -> Result<Minted, MintError> {
    let unavailable = |m: &str| MintError::Unavailable(m.to_owned());
    let (Some(client_id), Some(secret), Some(url)) = (
        text(made, &["key", "client_id"]),
        text(made, &["secret"]),
        text(made, &["token_url"]),
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
    let token = ClientCredentials::new(client_id, Secret::new(secret), scopes.iter().cloned())
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

fn text(v: &Value, path: &[&str]) -> Option<String> {
    let mut at = v;
    for k in path {
        at = at.get(*k)?;
    }
    at.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
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

    use super::narrowed;
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
}
