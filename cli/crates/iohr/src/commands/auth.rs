//! `iohr auth token`: a profile's access token for another program (`docs/config.md`
//! section 5.4).

use iohr_auth::{Claims, Credential as _};
use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::Env;
use crate::cli::{AuthToken, Global, TokenFormat};
use crate::context::session;
use crate::error::Error;
use crate::output::Out;

/// The JSON line, borrowing the token so it is copied only into the output buffer.
#[derive(Serialize)]
struct Printed<'a> {
    access_token: &'a str,
    expires_at: Option<String>,
    profile: Option<&'a str>,
    account: &'a str,
}

pub(crate) async fn token(g: &Global, env: &Env, args: &AuthToken, out: Out) -> Result<(), Error> {
    let s = session(g, env).await?;
    // A person's session refreshes here when less than a minute is left, with the
    // command line's own rules (re-reading the store, rotation).
    let bearer = s.credential.bearer().await?;
    let claims = Claims::read(bearer.expose(), OffsetDateTime::now_utc())?;
    let expires_at = claims
        .expires_at()
        .map(|t| t.format(&Rfc3339))
        .transpose()
        .map_err(|e| Error::Failed(format!("cannot write the expiry: {e}")))?;
    let account = if s.account.is_empty() {
        claims.org.as_deref().unwrap_or_default()
    } else {
        s.account.as_str()
    };
    let mut line = if out.json || args.format == TokenFormat::Json {
        serde_json::to_string(&Printed {
            access_token: bearer.expose(),
            expires_at,
            profile: s.profile.as_ref().map(iohr_auth::ProfileName::as_str),
            account,
        })
        .map_err(|e| Error::Failed(e.to_string()))?
    } else {
        bearer.expose().to_owned()
    };
    line.push('\n');
    Out::raw(line.as_bytes());
    zeroize::Zeroize::zeroize(&mut line);
    Ok(())
}
