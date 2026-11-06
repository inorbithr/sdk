//! Signing in as Io (RFC 0065.2): the `OAuth2` client `iohr-io` with the client credentials
//! grant, so jobs Io files run on runners Io owns. Opt-in for `iohr runner` with
//! `--sign-in io` (`IOHR_SIGN_IN=io`); the default sign-in is untouched.
//!
//! The minting, the in-memory cache and the early refresh are `inorbithr::ClientCredentials`,
//! the one implementation the SDK and `iohr ext` already use: this module adds only where
//! Io's secret comes from. The secret is never an argument or an environment value:
//!
//! - a file named by `IOHR_CLIENT_SECRET_FILE`, read once at start, which must belong to
//!   this user and be closed to group and world (mode 0600); or
//! - on macOS, the keychain item the Mac bridge uses (service `iohr-mcp-bridge-io`,
//!   account `inorbit`, the secret stored base64 encoded), read through `security`.
//!
//! The secret and the tokens live in memory only: nothing here writes either to disk or to
//! a log, and every message names a path or a client, never a value.

use std::path::{Path, PathBuf};

use inorbithr::{ClientCredentials, Secret};

use crate::error::Error;

/// The `OAuth2` client Io signs in as (the Mac bridge's `IO_CLIENT_ID`).
pub(crate) const IO_CLIENT_ID: &str = "iohr-io";
/// The keychain service of Io's secret (the Mac bridge's `KC_IO`).
pub(crate) const IO_KEYCHAIN_SERVICE: &str = "iohr-mcp-bridge-io";
/// The keychain account every bridge item uses.
pub(crate) const IO_KEYCHAIN_ACCOUNT: &str = "inorbit";
/// The variable naming the file that holds Io's secret.
pub(crate) const SECRET_FILE_VAR: &str = "IOHR_CLIENT_SECRET_FILE";
/// What a runner asks for: its own calls (leases, turns, reports), the coordinator's
/// reads and writes around them and the approvals its host actions use. Fewer than the
/// client holds; `iohr-io` is granted these.
pub(crate) const IO_SCOPES: &[&str] = &[
    "coordinator:runner",
    "coordinator:read",
    "coordinator:write",
    "approvals:read",
    "approvals:write",
];

/// Where Io's secret is read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SecretSource {
    File(PathBuf),
    Keychain,
}

impl SecretSource {
    /// The file `IOHR_CLIENT_SECRET_FILE` names, else the keychain on macOS.
    ///
    /// # Errors
    ///
    /// [`Error::NotSignedIn`] where there is neither.
    pub(crate) fn choose(file: Option<&str>) -> Result<Self, Error> {
        if let Some(p) = file.map(str::trim).filter(|p| !p.is_empty()) {
            return Ok(Self::File(PathBuf::from(p)));
        }
        if cfg!(target_os = "macos") {
            return Ok(Self::Keychain);
        }
        Err(Error::NotSignedIn(format!(
            "signing in as Io needs its secret in a file: set {SECRET_FILE_VAR} to the path of a \
             file only this user can read (chmod 600); the secret is never taken from an \
             argument or an environment value"
        )))
    }

    /// Reads the secret. The value goes straight into a [`Secret`].
    pub(crate) async fn read(&self) -> Result<Secret<String>, Error> {
        match self {
            Self::File(path) => {
                let p = path.clone();
                tokio::task::spawn_blocking(move || read_private_file(&p))
                    .await
                    .map_err(|_| Error::Failed("reading Io's secret file stopped".into()))?
            }
            Self::Keychain => read_keychain().await,
        }
    }
}

/// The credential that mints Io's token at `issuer`'s token endpoint, on first use and
/// again shortly before each 15-minute token ends.
///
/// # Errors
///
/// [`Error::Usage`] when `issuer` is not https (plain http is for this machine only).
pub(crate) fn credential(issuer: &str, secret: Secret<String>) -> Result<ClientCredentials, Error> {
    let url = url::Url::parse(&format!("{}/oauth2/token", issuer.trim_end_matches('/')))
        .map_err(|_| Error::Usage("the sign-in service address is not a URL".into()))?;
    if !(url.scheme() == "https" || (url.scheme() == "http" && crate::ext::narrow::loopback(&url)))
    {
        return Err(Error::Usage(
            "the sign-in service must be https (plain http only to this machine)".into(),
        ));
    }
    Ok(
        ClientCredentials::new(IO_CLIENT_ID, secret, IO_SCOPES.iter().copied())
            .map_err(|e| Error::Failed(format!("the sign-in could not start: {e}")))?
            .token_url(url),
    )
}

/// Reads `path`, refusing it unless it is a regular file this user owns that group and
/// world cannot touch. The file is opened once and its owner and mode are read from that
/// handle, so what is checked is what is read.
pub(crate) fn read_private_file(path: &Path) -> Result<Secret<String>, Error> {
    use std::io::Read as _;
    let name = path.display();
    let mut file = std::fs::File::open(path)
        .map_err(|e| Error::NotSignedIn(format!("cannot open {name}: {}", e.kind())))?;
    let meta = file
        .metadata()
        .map_err(|e| Error::NotSignedIn(format!("cannot read {name}: {}", e.kind())))?;
    if !meta.is_file() {
        return Err(Error::NotSignedIn(format!("{name} is not a regular file")));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let me = rustix::process::geteuid().as_raw();
        if meta.uid() != me {
            return Err(Error::NotSignedIn(format!(
                "{name} is owned by another user (uid {}, not {me}); refused",
                meta.uid()
            )));
        }
        let mode = meta.mode() & 0o7777;
        if mode & 0o077 != 0 {
            return Err(Error::NotSignedIn(format!(
                "{name} can be read by group or world (mode {mode:04o}); refused, run chmod 600 on it"
            )));
        }
    }
    // The secret is a short line: a file of any size past this is not one.
    let mut text = String::new();
    (&mut file)
        .take(4096)
        .read_to_string(&mut text)
        .map_err(|e| Error::NotSignedIn(format!("cannot read {name}: {}", e.kind())))?;
    let secret = text.trim();
    if secret.is_empty() {
        return Err(Error::NotSignedIn(format!("{name} is empty")));
    }
    Ok(Secret::new(secret.to_owned()))
}

/// Io's secret from the login keychain: `security find-generic-password -w`, the same
/// lookup the Mac bridge makes. The item holds the secret base64 encoded.
async fn read_keychain() -> Result<Secret<String>, Error> {
    use base64::Engine as _;
    let missing = || {
        Error::NotSignedIn(format!(
            "Io's secret is not in the keychain (item {IO_KEYCHAIN_SERVICE}): the owner's ask \
             script installs it, or set {SECRET_FILE_VAR}"
        ))
    };
    let out = tokio::process::Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-s",
            IO_KEYCHAIN_SERVICE,
            "-a",
            IO_KEYCHAIN_ACCOUNT,
            "-w",
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|_| missing())?;
    if !out.status.success() {
        return Err(missing());
    }
    let stored = String::from_utf8_lossy(&out.stdout);
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(stored.trim())
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .filter(|s| !s.is_empty())
        .ok_or_else(missing)?;
    Ok(Secret::new(decoded))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn file(mode: u32, body: &str) -> (tempfile::TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("io-secret");
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        (dir, path)
    }

    #[cfg(unix)]
    #[test]
    fn a_private_file_is_read_and_trimmed() {
        let (_d, p) = file(0o600, "  S3CR3T-MARKER\n");
        assert_eq!(read_private_file(&p).unwrap().expose(), "S3CR3T-MARKER");
    }

    #[cfg(unix)]
    #[test]
    fn a_file_others_can_read_is_refused_without_its_content() {
        for mode in [0o640, 0o604, 0o644, 0o660] {
            let (_d, p) = file(mode, "S3CR3T-MARKER");
            let e = read_private_file(&p).unwrap_err().to_string();
            assert!(e.contains("refused"), "{e}");
            assert!(!e.contains("S3CR3T-MARKER"), "{e}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_empty_or_missing_file_is_refused() {
        let (_d, p) = file(0o600, "  \n");
        assert!(
            read_private_file(&p)
                .unwrap_err()
                .to_string()
                .contains("empty")
        );
        assert!(read_private_file(Path::new("/nonexistent/io-secret")).is_err());
    }

    #[test]
    fn the_file_variable_wins_and_a_blank_one_is_no_file() {
        assert_eq!(
            SecretSource::choose(Some("/run/secrets/io")).unwrap(),
            SecretSource::File("/run/secrets/io".into())
        );
        let fallback = SecretSource::choose(Some("  "));
        if cfg!(target_os = "macos") {
            assert_eq!(fallback.unwrap(), SecretSource::Keychain);
        } else {
            let e = fallback.unwrap_err().to_string();
            assert!(e.contains(SECRET_FILE_VAR), "{e}");
        }
    }

    #[test]
    fn the_token_endpoint_must_be_https_or_loopback() {
        let s = || Secret::new("x".to_owned());
        assert!(credential("https://auth.inorbit.hr", s()).is_ok());
        assert!(credential("http://127.0.0.1:4444/", s()).is_ok());
        assert!(credential("http://auth.inorbit.hr", s()).is_err());
        assert!(credential("not a url", s()).is_err());
    }

    use crate::context::AnyCredential;
    use iohr_auth::Credential as _;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SECRET: &str = "S3CR3T-MARKER";

    fn io(server: &MockServer) -> AnyCredential {
        AnyCredential::Io(Box::new(
            credential(&server.uri(), Secret::new(SECRET.to_owned())).unwrap(),
        ))
    }

    fn answer(token: &str, expires_in: u64) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": token, "token_type": "bearer", "expires_in": expires_in,
        }))
    }

    /// The first call mints with the `iohr-io` client over HTTP Basic (the secret in the
    /// header, never the body), the audience and the runner's scopes; the next calls use
    /// the cached token without another request.
    #[tokio::test]
    async fn it_mints_once_with_basic_auth_and_then_uses_the_cache() {
        use base64::Engine as _;
        let server = MockServer::start().await;
        let basic = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{IO_CLIENT_ID}:{SECRET}"))
        );
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .and(header("authorization", basic.as_str()))
            .and(body_string_contains("grant_type=client_credentials"))
            .and(body_string_contains("audience=iohr-api"))
            .and(body_string_contains("coordinator%3Arunner"))
            .respond_with(answer("tok-1", 900))
            .expect(1)
            .mount(&server)
            .await;
        let c = io(&server);
        for _ in 0..3 {
            assert_eq!(c.bearer().await.unwrap().expose(), "tok-1");
        }
        let seen = server.received_requests().await.unwrap();
        assert_eq!(seen.len(), 1);
        assert!(
            !String::from_utf8_lossy(&seen[0].body).contains(SECRET),
            "the secret must not travel in the body"
        );
    }

    /// A token with a second to live is minted again before it ends, and the new one is
    /// handed out.
    #[tokio::test]
    async fn it_mints_again_shortly_before_the_token_ends() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .respond_with(answer("tok-1", 1))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .respond_with(answer("tok-2", 900))
            .mount(&server)
            .await;
        let c = io(&server);
        assert_eq!(c.bearer().await.unwrap().expose(), "tok-1");
        tokio::time::sleep(std::time::Duration::from_millis(900)).await;
        assert_eq!(c.bearer().await.unwrap().expose(), "tok-2");
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    /// The API refuses a token the cache still holds (a Mac that slept): the one retry
    /// after the 401 must mint a fresh token, through the same `Api` the runner uses.
    #[tokio::test]
    async fn a_401_makes_the_api_mint_a_fresh_token_and_retry() {
        use std::sync::Arc;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .respond_with(answer("tok-old", 900))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .respond_with(answer("tok-new", 900))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/ping"))
            .and(header("authorization", "Bearer tok-new"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/ping"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "code": "unauthenticated", "error": "a valid bearer token is required", "details": [],
            })))
            .mount(&server)
            .await;
        let api = crate::context::Api::new(&server.uri(), Arc::new(io(&server)), false).unwrap();
        let _: serde_json::Value = api.get("/v1/ping", &[]).await.unwrap();
        let mints = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path() == "/oauth2/token")
            .count();
        assert_eq!(mints, 2, "the 401 must mint again");
    }

    /// A refused sign-in is an error that names the client and the reason, never the
    /// secret, and neither does the credential's `Debug`.
    #[tokio::test]
    async fn an_error_and_the_debug_output_never_contain_the_secret() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "error": "invalid_client",
                "error_description": "Client authentication failed",
            })))
            .mount(&server)
            .await;
        let c = io(&server);
        let e = c.bearer().await.unwrap_err().to_string();
        assert!(e.contains("invalid_client"), "{e}");
        assert!(!e.contains(SECRET), "{e}");
        assert!(!format!("{c:?}").contains(SECRET));
    }
}
