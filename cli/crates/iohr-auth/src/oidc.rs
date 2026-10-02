//! The identity provider: discovery, the token endpoint, revocation.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use time::OffsetDateTime;
use url::Url;

use crate::claims::Claims;
use crate::error::AuthError;
use crate::secret::Redacted;

/// The InOrbit sign-in service.
pub const DEFAULT_ISSUER: &str = "https://auth.inorbit.hr";
/// The command line's public OAuth client: no secret, PKCE required.
pub const DEFAULT_CLIENT_ID: &str = "iohr-cli";
/// What a person's session asks for: an ID token, a refresh token, the API.
pub(crate) const SCOPES: &str = "openid offline_access iohr.api";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// The largest answer read from the sign-in service.
const MAX_BODY: usize = 256 * 1024;

/// A discovered identity provider and the client the command line signs in as.
#[derive(Debug, Clone)]
pub struct Provider {
    pub(crate) http: reqwest::Client,
    issuer: String,
    client_id: String,
    pub(crate) authorization_endpoint: Url,
    pub(crate) token_endpoint: Url,
    pub(crate) device_authorization_endpoint: Option<Url>,
    pub(crate) revocation_endpoint: Option<Url>,
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    device_authorization_endpoint: Option<String>,
    #[serde(default)]
    revocation_endpoint: Option<String>,
}

impl Provider {
    /// Reads `issuer`'s OpenID configuration and checks it: the document names the
    /// same issuer, and every endpoint is on the issuer's origin.
    ///
    /// # Errors
    ///
    /// [`AuthError::Discovery`] when the issuer URL is not HTTPS (loopback excepted),
    /// the document cannot be fetched, or it fails a check.
    pub async fn discover(issuer: &str, client_id: &str) -> Result<Self, AuthError> {
        let base = checked_origin(issuer)?;
        let http = reqwest::Client::builder()
            .user_agent(concat!("iohr/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .https_only(base.scheme() == "https")
            .build()
            .map_err(|e| AuthError::Discovery(format!("cannot start the HTTP client: {e}")))?;
        let url = format!(
            "{}/.well-known/openid-configuration",
            issuer.trim_end_matches('/')
        );
        let resp = http
            .get(&url)
            .send()
            .await
            .map_err(|e| AuthError::Discovery(reason(&e)))?;
        if !resp.status().is_success() {
            return Err(AuthError::Discovery(format!(
                "{url} answered HTTP {}",
                resp.status().as_u16()
            )));
        }
        let doc: Discovery = serde_json::from_slice(&read_capped(resp).await?).map_err(|e| {
            AuthError::Discovery(format!("the OpenID configuration is not valid: {e}"))
        })?;
        // RFC 8414 section 3.3: the issuer in the document is the one asked for.
        if doc.issuer != issuer {
            return Err(AuthError::Discovery(format!(
                "the provider says it is {}, not {issuer}",
                doc.issuer
            )));
        }
        let endpoint = |raw: &str| -> Result<Url, AuthError> {
            let u =
                Url::parse(raw).map_err(|_| AuthError::Discovery(format!("{raw} is not a URL")))?;
            if u.origin() == base.origin() {
                Ok(u)
            } else {
                Err(AuthError::Discovery(format!("{raw} is not on {issuer}")))
            }
        };
        Ok(Self {
            authorization_endpoint: endpoint(&doc.authorization_endpoint)?,
            token_endpoint: endpoint(&doc.token_endpoint)?,
            device_authorization_endpoint: doc
                .device_authorization_endpoint
                .as_deref()
                .map(endpoint)
                .transpose()?,
            revocation_endpoint: doc
                .revocation_endpoint
                .as_deref()
                .map(endpoint)
                .transpose()?,
            http,
            issuer: issuer.to_owned(),
            client_id: client_id.to_owned(),
        })
    }

    /// The issuer, as configured and confirmed by discovery.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// The OAuth client id.
    #[must_use]
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// `raw` as a URL when it is on the provider's origin, as every link it gives a
    /// person to open must be.
    pub(crate) fn same_origin(&self, raw: &str) -> Option<Url> {
        let u = Url::parse(raw).ok()?;
        (u.origin() == self.token_endpoint.origin()).then_some(u)
    }

    /// One POST to the token endpoint.
    pub(crate) async fn token(&self, form: &[(&str, &str)]) -> Result<TokenResponse, TokenError> {
        let resp = self
            .http
            .post(self.token_endpoint.clone())
            .form(form)
            .send()
            .await
            .map_err(|e| TokenError::Transport(reason(&e)))?;
        let status = resp.status();
        let body = read_capped(resp)
            .await
            .map_err(|e| TokenError::Transport(e.to_string()))?;
        if status.is_success() {
            return serde_json::from_slice(&body).map_err(|_| {
                TokenError::Transport(
                    "the token endpoint answered something that is not a token".into(),
                )
            });
        }
        match serde_json::from_slice::<OAuthErrorWire>(&body) {
            Ok(e) => Err(TokenError::OAuth {
                error: e.error,
                description: e.error_description.unwrap_or_default(),
            }),
            Err(_) => Err(TokenError::Transport(format!(
                "the token endpoint answered HTTP {}",
                status.as_u16()
            ))),
        }
    }

    /// Revokes a refresh token (RFC 7009). Revoking an unknown token succeeds.
    ///
    /// # Errors
    ///
    /// [`AuthError::Http`] when the provider has no revocation endpoint or the call
    /// fails.
    pub async fn revoke(&self, refresh_token: &Redacted<String>) -> Result<(), AuthError> {
        let url = self
            .revocation_endpoint
            .clone()
            .ok_or_else(|| AuthError::Http("the provider cannot revoke tokens".into()))?;
        let resp = self
            .http
            .post(url)
            .form(&[
                ("token", refresh_token.expose().as_str()),
                ("token_type_hint", "refresh_token"),
                ("client_id", self.client_id.as_str()),
            ])
            .send()
            .await
            .map_err(|e| AuthError::Http(reason(&e)))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(AuthError::Http(format!(
                "revoking answered HTTP {}",
                resp.status().as_u16()
            )))
        }
    }

    /// Checks a token answer: a bearer access token for the API, a refresh token when
    /// `need_refresh`, and an ID token from this issuer for this client when
    /// `need_id_token`. The ID token comes straight from the token endpoint over TLS,
    /// which is what lets its signature go unchecked (OpenID Connect Core 3.1.3.7).
    pub(crate) fn tokens(
        &self,
        wire: TokenResponse,
        need_refresh: bool,
        need_id_token: bool,
    ) -> Result<Tokens, AuthError> {
        if !wire.token_type.eq_ignore_ascii_case("bearer") {
            return Err(AuthError::Http(format!(
                "unexpected token type {}",
                wire.token_type
            )));
        }
        let now = OffsetDateTime::now_utc();
        let claims = Claims::read(wire.access_token.expose(), now)?;
        match (&wire.id_token, need_id_token) {
            (Some(id), _) => self.check_id_token(id)?,
            (None, true) => return Err(AuthError::Http("the sign-in returned no ID token".into())),
            (None, false) => {}
        }
        if need_refresh && wire.refresh_token.is_none() {
            return Err(AuthError::Http(
                "the sign-in returned no refresh token (offline_access)".into(),
            ));
        }
        let expires_at = wire
            .expires_in
            .map(|s| now + time::Duration::seconds(s))
            .or_else(|| claims.expires_at())
            .unwrap_or(now);
        Ok(Tokens {
            access: wire.access_token,
            refresh: wire.refresh_token,
            expires_at,
            claims,
        })
    }

    fn check_id_token(&self, id: &Redacted<String>) -> Result<(), AuthError> {
        #[derive(Deserialize)]
        struct IdClaims {
            iss: String,
            #[serde(deserialize_with = "crate::claims::audiences")]
            aud: Vec<String>,
        }
        let bad = || AuthError::Http("the ID token cannot be read".into());
        let payload = id.expose().split('.').nth(1).ok_or_else(bad)?;
        let bytes = URL_SAFE_NO_PAD
            .decode(payload.trim_end_matches('='))
            .map_err(|_| bad())?;
        let c: IdClaims = serde_json::from_slice(&bytes).map_err(|_| bad())?;
        if c.iss != self.issuer {
            return Err(AuthError::IssuerMismatch);
        }
        if !c.aud.iter().any(|a| a == &self.client_id) {
            return Err(AuthError::Http("the ID token is for another client".into()));
        }
        Ok(())
    }
}

/// HTTPS, or HTTP to this machine, with nothing but an origin and an optional path.
fn checked_origin(issuer: &str) -> Result<Url, AuthError> {
    let u =
        Url::parse(issuer).map_err(|_| AuthError::Discovery(format!("{issuer} is not a URL")))?;
    let loopback = match u.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(d)) => d == "localhost",
        None => false,
    };
    let scheme_ok = u.scheme() == "https" || (u.scheme() == "http" && loopback);
    if !scheme_ok || u.query().is_some() || u.fragment().is_some() || !u.username().is_empty() {
        return Err(AuthError::Discovery(format!(
            "{issuer} must be an https URL (plain http only to this machine)"
        )));
    }
    Ok(u)
}

async fn read_capped(mut resp: reqwest::Response) -> Result<Vec<u8>, AuthError> {
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| AuthError::Http(reason(&e)))?
    {
        if body.len() + chunk.len() > MAX_BODY {
            return Err(AuthError::Http(
                "the sign-in service answered too much".into(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// The error's causes without the URL, which may carry a code.
pub(crate) fn reason(e: &reqwest::Error) -> String {
    let mut parts = Vec::new();
    let mut source: Option<&dyn std::error::Error> = std::error::Error::source(e);
    while let Some(s) = source {
        parts.push(s.to_string());
        source = s.source();
    }
    if e.is_timeout() {
        parts.insert(0, "timed out".into());
    }
    if parts.is_empty() {
        "the connection failed".into()
    } else {
        parts.join(": ")
    }
}

/// A token endpoint answer. Its secrets are [`Redacted`] from the moment they are read.
#[derive(Deserialize)]
pub(crate) struct TokenResponse {
    pub(crate) access_token: Redacted<String>,
    pub(crate) token_type: String,
    #[serde(default)]
    pub(crate) expires_in: Option<i64>,
    #[serde(default)]
    pub(crate) refresh_token: Option<Redacted<String>>,
    #[serde(default)]
    pub(crate) id_token: Option<Redacted<String>>,
}

#[derive(Deserialize)]
struct OAuthErrorWire {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// Why a token request failed.
#[derive(Debug)]
pub(crate) enum TokenError {
    /// An OAuth error answer, such as `authorization_pending` or `invalid_grant`.
    OAuth { error: String, description: String },
    /// No usable answer.
    Transport(String),
}

impl From<TokenError> for AuthError {
    fn from(e: TokenError) -> Self {
        match e {
            TokenError::OAuth { error, .. } if error == "access_denied" => Self::Denied,
            TokenError::OAuth { error, description } => Self::OAuth { error, description },
            TokenError::Transport(m) => Self::Http(m),
        }
    }
}

/// Checked tokens from a grant or a refresh.
pub(crate) struct Tokens {
    pub(crate) access: Redacted<String>,
    pub(crate) refresh: Option<Redacted<String>>,
    pub(crate) expires_at: OffsetDateTime,
    pub(crate) claims: Claims,
}

#[cfg(test)]
mod tests {
    use super::checked_origin;

    #[test]
    fn issuers_are_https_or_loopback() {
        for ok in [
            "https://auth.inorbit.hr",
            "http://127.0.0.1:4444",
            "http://[::1]:4444",
            "http://localhost:4444",
        ] {
            assert!(checked_origin(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://auth.inorbit.hr",
            "https://auth.inorbit.hr?x=1",
            "https://u@auth.inorbit.hr",
            "nope",
        ] {
            assert!(checked_origin(bad).is_err(), "{bad}");
        }
    }
}
