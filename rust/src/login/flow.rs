//! Signing a person in: in a browser on this machine, or with a device code.
//!
//! Each grant is a typestate. A browser sign-in is an `Authorization<Browser>` until
//! the browser comes back, a device sign-in an `Authorization<Device>` until the code
//! is approved; both end as `Authorization<Granted>`, and only that state can become a
//! stored [`Session`](super::Session). A half-finished sign-in cannot be saved.

use std::time::Duration;

use tokio::time::Instant;
use url::Url;

use super::loopback::{Callback, Listener, SIGN_IN_WINDOW};
use super::page::Page;
use super::provider::{Provider, TokenError, Tokens, read_answer};
use super::{AUDIENCE, LoginError, Pkce, constant_time_eq, random_token};
use crate::secret::Secret;

/// The longest a device sign-in waits, whatever the provider allows.
const DEVICE_WINDOW: Duration = Duration::from_mins(15);
/// RFC 8628 section 3.5: wait 5 s more after each `slow_down`.
const SLOW_DOWN_STEP: Duration = Duration::from_secs(5);
/// Never poll faster than this, whatever the provider says.
const MIN_INTERVAL: Duration = Duration::from_secs(1);

/// Where a sign-in starts: `Login::browser(&provider)` or `Login::device(&provider)`,
/// then `.finish().await`, then [`Session::save`](super::Session::save).
#[derive(Debug, Clone, Copy)]
pub struct Login;

impl Login {
    /// A sign-in in a browser on this machine: binds the loopback listener and builds
    /// the URL to open (see [`open_browser`](super::open_browser)).
    ///
    /// # Errors
    ///
    /// See [`Authorization::<Browser>::start`].
    pub async fn browser(provider: &Provider) -> Result<Authorization<Browser>, LoginError> {
        Authorization::<Browser>::start(provider.clone()).await
    }

    /// A sign-in with a device code, for a machine with no browser: show
    /// [`user_code`](Authorization::user_code) and
    /// [`verification_uri`](Authorization::verification_uri), never the device code.
    ///
    /// # Errors
    ///
    /// See [`Authorization::<Device>::start`].
    pub async fn device(provider: &Provider) -> Result<Authorization<Device>, LoginError> {
        Authorization::<Device>::start(provider.clone()).await
    }
}

/// A sign-in in one of its states.
#[derive(Debug)]
pub struct Authorization<S> {
    provider: Provider,
    state: S,
}

/// Waiting for the browser to come back to the loopback listener.
#[derive(Debug)]
pub struct Browser {
    url: Url,
    listener: Listener,
    pkce: Pkce,
    state: Secret<String>,
    profile: Option<String>,
}

/// Waiting for a person to approve a code on another device.
#[derive(Debug)]
pub struct Device {
    code: Secret<String>,
    user_code: String,
    verification_uri: Url,
    verification_uri_complete: Option<Url>,
    interval: Duration,
    deadline: Instant,
}

/// Signed in: tokens checked and ready to be stored.
pub struct Granted {
    pub(crate) tokens: Tokens,
}

impl std::fmt::Debug for Granted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Granted")
            .field("account", &self.tokens.claims.org)
            .finish_non_exhaustive()
    }
}

impl<S> Authorization<S> {
    /// The provider this sign-in is with.
    #[must_use]
    pub fn provider(&self) -> &Provider {
        &self.provider
    }
}

impl Authorization<Browser> {
    /// Binds the loopback listener and builds the authorization URL: code flow, PKCE
    /// S256, a fresh `state`, `audience=iohr-api`.
    ///
    /// # Errors
    ///
    /// [`LoginError::Callback`] when no loopback port can be bound, and
    /// [`LoginError::Random`] when the system has no randomness.
    pub async fn start(provider: Provider) -> Result<Self, LoginError> {
        let listener = Listener::bind()
            .await
            .map_err(|e| LoginError::Callback(format!("cannot listen on this machine: {e}")))?;
        let pkce = Pkce::new()?;
        let state = random_token()?;
        let mut url = provider.authorization_endpoint.clone();
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", provider.client_id())
            .append_pair("redirect_uri", listener.redirect_uri())
            .append_pair("scope", &provider.scope())
            .append_pair("audience", AUDIENCE)
            .append_pair("state", state.expose())
            .append_pair("code_challenge", &pkce.challenge)
            .append_pair("code_challenge_method", "S256");
        Ok(Self {
            provider,
            state: Browser {
                url,
                listener,
                pkce,
                state,
                profile: None,
            },
        })
    }

    /// Names the profile being signed in, for the page the browser lands on.
    #[must_use]
    pub fn for_profile(mut self, profile: impl Into<String>) -> Self {
        self.state.profile = Some(profile.into());
        self
    }

    /// The URL to open in the browser.
    #[must_use]
    pub fn url(&self) -> &Url {
        &self.state.url
    }

    /// Waits for the browser (at most 5 minutes), checks the answer, and exchanges the
    /// code for tokens.
    ///
    /// # Errors
    ///
    /// [`LoginError::TimedOut`], [`LoginError::Denied`], [`LoginError::StateMismatch`],
    /// [`LoginError::IssuerMismatch`], [`LoginError::Callback`], or the token exchange's
    /// error.
    pub async fn finish(self) -> Result<Authorization<Granted>, LoginError> {
        self.finish_within(SIGN_IN_WINDOW).await
    }

    /// [`finish`](Self::finish) with another window, for tests.
    #[doc(hidden)]
    pub async fn finish_within(
        self,
        window: Duration,
    ) -> Result<Authorization<Granted>, LoginError> {
        let Self {
            provider,
            state:
                Browser {
                    listener,
                    pkce,
                    state,
                    profile,
                    ..
                },
        } = self;
        let redirect_uri = listener.redirect_uri().to_owned();
        let (cb, reply) = listener.accept_callback(window).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::TimedOut {
                LoginError::TimedOut
            } else {
                LoginError::Callback(e.to_string())
            }
        })?;
        // The browser waits on its connection until the outcome is known, so its page
        // says what really happened.
        let code = match verdict(&cb, state.expose(), provider.issuer()) {
            Ok(code) => code,
            Err(e) => {
                reply.send(failure_page(&e)).await;
                return Err(e);
            }
        };
        let exchanged = async {
            let wire = provider
                .token(&[
                    ("grant_type", "authorization_code"),
                    ("code", code.expose()),
                    ("redirect_uri", &redirect_uri),
                    ("client_id", provider.client_id()),
                    ("code_verifier", pkce.verifier.expose()),
                ])
                .await?;
            provider.tokens(wire, true, true)
        }
        .await;
        let tokens = match exchanged {
            Ok(tokens) => tokens,
            Err(e) => {
                reply.send(failure_page(&e)).await;
                return Err(e);
            }
        };
        reply
            .send(Page::SignedIn {
                profile: profile.as_deref(),
                account: tokens.claims.org.as_deref(),
                plan: tokens.claims.plan.as_deref(),
            })
            .await;
        Ok(Authorization {
            provider,
            state: Granted { tokens },
        })
    }
}

/// The page for a sign-in that failed: a fixed sentence per kind of failure, never
/// text from the request or the sign-in service.
fn failure_page(e: &LoginError) -> Page<'static> {
    match e {
        LoginError::Denied => Page::Denied,
        LoginError::StateMismatch => Page::StateMismatch,
        LoginError::IssuerMismatch => {
            Page::Failed("The answer came from another sign-in service than the one configured.")
        }
        LoginError::OAuth { .. } => Page::Failed("The sign-in service refused the request."),
        LoginError::Callback(_) => Page::Failed("The browser came back without a sign-in code."),
        _ => Page::Failed("The sign-in code could not be exchanged for a session."),
    }
}

/// The code from a callback, or why there is none. `state` is compared in constant
/// time; an `iss` parameter, when present, must be the issuer (RFC 9207).
fn verdict(
    cb: &Callback,
    expected_state: &str,
    issuer: &str,
) -> Result<Secret<String>, LoginError> {
    let state_ok = cb
        .state
        .as_deref()
        .is_some_and(|s| constant_time_eq(s.as_bytes(), expected_state.as_bytes()));
    if !state_ok {
        return Err(LoginError::StateMismatch);
    }
    if cb.iss.as_deref().is_some_and(|iss| iss != issuer) {
        return Err(LoginError::IssuerMismatch);
    }
    match (&cb.error, &cb.code) {
        (Some(e), _) if e == "access_denied" => Err(LoginError::Denied),
        (Some(e), _) => Err(LoginError::OAuth {
            error: e.clone(),
            description: cb.error_description.clone().unwrap_or_default(),
        }),
        (None, Some(code)) if !code.is_empty() => Ok(Secret::new(code.clone())),
        (None, _) => Err(LoginError::Callback(
            "the browser came back without a code".into(),
        )),
    }
}

impl Authorization<Device> {
    /// Asks the provider for a device code.
    ///
    /// # Errors
    ///
    /// [`LoginError::NoDeviceGrant`] when the provider has no device endpoint, and
    /// [`LoginError::Http`] when the request fails or the answer is unusable.
    pub async fn start(provider: Provider) -> Result<Self, LoginError> {
        #[derive(serde::Deserialize)]
        struct Wire {
            device_code: Secret<String>,
            user_code: String,
            verification_uri: String,
            #[serde(default)]
            verification_uri_complete: Option<String>,
            expires_in: u64,
            #[serde(default)]
            interval: Option<u64>,
        }
        let endpoint = provider
            .device_authorization_endpoint
            .clone()
            .ok_or(LoginError::NoDeviceGrant)?;
        let resp = provider
            .http
            .post(endpoint)
            .form(&[
                ("client_id", provider.client_id()),
                ("scope", provider.scope().as_str()),
                ("audience", AUDIENCE),
            ])
            .send()
            .await
            .map_err(|e| LoginError::Http(super::provider::reason(&e)))?;
        if !resp.status().is_success() {
            return Err(LoginError::Http(format!(
                "the device code request answered HTTP {}",
                resp.status().as_u16()
            )));
        }
        let w: Wire = serde_json::from_slice(&read_answer(resp).await?)
            .map_err(|_| LoginError::Http("the device code answer cannot be read".into()))?;
        // Every link shown to a person is on the sign-in service itself.
        let verification_uri = provider
            .same_origin(&w.verification_uri)
            .ok_or_else(|| LoginError::Http("the device code points to another site".into()))?;
        let verification_uri_complete = w
            .verification_uri_complete
            .as_deref()
            .and_then(|u| provider.same_origin(u));
        let window = Duration::from_secs(w.expires_in).min(DEVICE_WINDOW);
        let interval = Duration::from_secs(w.interval.unwrap_or(5)).max(MIN_INTERVAL);
        Ok(Self {
            provider,
            state: Device {
                code: w.device_code,
                user_code: w.user_code,
                verification_uri,
                verification_uri_complete,
                interval,
                deadline: Instant::now() + window,
            },
        })
    }

    /// The code the person types.
    #[must_use]
    pub fn user_code(&self) -> &str {
        &self.state.user_code
    }

    /// Where the person types it.
    #[must_use]
    pub fn verification_uri(&self) -> &Url {
        &self.state.verification_uri
    }

    /// The same page with the code filled in, when the provider offers one.
    #[must_use]
    pub fn verification_uri_complete(&self) -> Option<&Url> {
        self.state.verification_uri_complete.as_ref()
    }

    /// Polls the token endpoint until the code is approved, denied or expired,
    /// honouring `interval` and `slow_down` (RFC 8628 section 3.5).
    ///
    /// # Errors
    ///
    /// [`LoginError::Denied`] when the person refuses, [`LoginError::TimedOut`] when the
    /// code expires, or the token endpoint's error.
    pub async fn finish(self) -> Result<Authorization<Granted>, LoginError> {
        let Self {
            provider,
            state: mut d,
        } = self;
        loop {
            if Instant::now() + d.interval > d.deadline {
                return Err(LoginError::TimedOut);
            }
            tokio::time::sleep(d.interval).await;
            let answer = provider
                .token(&[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("device_code", d.code.expose()),
                    ("client_id", provider.client_id()),
                ])
                .await;
            match answer {
                Ok(wire) => {
                    let tokens = provider.tokens(wire, true, true)?;
                    return Ok(Authorization {
                        provider,
                        state: Granted { tokens },
                    });
                }
                Err(TokenError::OAuth { error, .. }) if error == "authorization_pending" => {}
                Err(TokenError::OAuth { error, .. }) if error == "slow_down" => {
                    d.interval += SLOW_DOWN_STEP;
                }
                Err(TokenError::OAuth { error, .. }) if error == "expired_token" => {
                    return Err(LoginError::TimedOut);
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}

impl Authorization<Granted> {
    /// The account the session's calls count against.
    #[must_use]
    pub fn account(&self) -> Option<&str> {
        self.state.tokens.claims.org.as_deref()
    }

    /// The person's id.
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.state.tokens.claims.sub
    }

    pub(crate) fn into_parts(self) -> (Provider, Tokens) {
        (self.provider, self.state.tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::super::LoginError;
    use super::super::loopback::Callback;
    use super::verdict;

    fn cb(state: &str, code: Option<&str>, error: Option<&str>, iss: Option<&str>) -> Callback {
        Callback {
            state: Some(state.into()),
            code: code.map(Into::into),
            error: error.map(Into::into),
            iss: iss.map(Into::into),
            ..Callback::default()
        }
    }

    #[test]
    fn a_callback_is_checked_before_its_code_is_used() {
        let iss = "https://auth.example";
        assert_eq!(
            verdict(&cb("s", Some("c"), None, None), "s", iss)
                .unwrap()
                .expose(),
            "c"
        );
        assert_eq!(
            verdict(&cb("s", Some("c"), None, Some(iss)), "s", iss)
                .unwrap()
                .expose(),
            "c"
        );
        assert!(matches!(
            verdict(&cb("x", Some("c"), None, None), "s", iss),
            Err(LoginError::StateMismatch)
        ));
        assert!(matches!(
            verdict(&Callback::default(), "s", iss),
            Err(LoginError::StateMismatch)
        ));
        assert!(matches!(
            verdict(&cb("s", Some("c"), None, Some("https://evil")), "s", iss),
            Err(LoginError::IssuerMismatch)
        ));
        assert!(matches!(
            verdict(&cb("s", None, Some("access_denied"), None), "s", iss),
            Err(LoginError::Denied)
        ));
        assert!(matches!(
            verdict(&cb("s", None, Some("server_error"), None), "s", iss),
            Err(LoginError::OAuth { .. })
        ));
        assert!(matches!(
            verdict(&cb("s", None, None, None), "s", iss),
            Err(LoginError::Callback(_))
        ));
    }
}
