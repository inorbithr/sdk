//! `iohr <extension> ...`: run an installed extension as its own process, with the
//! token channel open for as long as it runs (ADR 0012).

use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use iohr_auth::{Claims, Credential as _, Redacted};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::OnceCell;

use super::install::{Store, check_confirmed, check_program};
use super::socket::{Ctx, MintError, Minted, Minter, handle};
use crate::Env;
use crate::cli::Global;
use crate::context::{Session, session};
use crate::error::Error;

/// Hands out the profile's access token, signing in (or refreshing) on first use.
pub(crate) struct ProfileMinter {
    global: Global,
    env: Env,
    session: OnceCell<Session>,
}

impl Minter for ProfileMinter {
    async fn mint(&self, scopes: Vec<String>) -> Result<Minted, MintError> {
        let s = self
            .session
            .get_or_try_init(|| session(&self.global, &self.env))
            .await
            .map_err(|e| MintError::Unavailable(e.to_string()))?;
        let bearer = s
            .credential
            .bearer()
            .await
            .map_err(|e| MintError::Unavailable(e.to_string()))?;
        let claims = Claims::read(bearer.expose(), OffsetDateTime::now_utc())
            .map_err(|e| MintError::Unavailable(e.to_string()))?;
        // The platform cannot narrow a person's token yet: the token is handed out only
        // when it already holds every scope asked for (ADR 0012).
        if let Some(missing) = scopes.iter().find(|s| !claims.scp.contains(s)) {
            return Err(MintError::Forbidden(format!(
                "profile {} does not hold {missing}",
                s.label
            )));
        }
        let expires_at = claims
            .exp
            .and_then(|e| OffsetDateTime::from_unix_timestamp(e).ok())
            .and_then(|t| t.format(&Rfc3339).ok())
            .unwrap_or_default();
        Ok(Minted {
            access_token: Redacted::new(bearer.expose().to_owned()),
            expires_at,
        })
    }
}

/// Runs extension `name` with `args`; returns its exit code.
pub(crate) async fn run(
    global: &Global,
    env: &Env,
    store: &Store,
    name: &str,
    args: &[OsString],
) -> Result<u8, Error> {
    let Some((entry, record, dir)) = store
        .installed(name)
        .map_err(|e| Error::Failed(e.to_string()))?
    else {
        return Err(Error::Usage(format!(
            "`{name}` is not an iohr command or an installed extension. `iohr --help` lists the \
             commands; `iohr ext install {name}` installs an extension by that name."
        )));
    };
    let program = check_program(&record, &dir).map_err(|e| Error::Failed(e.to_string()))?;
    check_confirmed(&entry, &record).map_err(|e| Error::Failed(e.to_string()))?;
    let base = api_origin(&global.base_url)?;
    let ctx = Arc::new(Ctx {
        minter: ProfileMinter {
            global: global.clone(),
            env: env.clone(),
            session: OnceCell::new(),
        },
        allowed: record.manifest.scopes.clone(),
        verbose: global.verbose,
    });
    serve(&program, base.as_str().trim_end_matches('/'), args, ctx).await
}

/// The API's origin for `IOHR_EXT_API`: HTTPS, or plain HTTP to this machine only
/// (SR-07), as the runtime's client accepts it.
fn api_origin(raw: &str) -> Result<url::Url, Error> {
    let url = url::Url::parse(raw)
        .map_err(|e| Error::Failed(format!("IOHR_BASE_URL is not a URL: {e}")))?;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(d)) => d == "localhost",
        None => false,
    };
    let ok = (url.scheme() == "https" || (url.scheme() == "http" && loopback))
        && url.path() == "/"
        && url.query().is_none()
        && url.username().is_empty()
        && url.password().is_none();
    if ok {
        Ok(url)
    } else {
        Err(Error::Failed(
            "IOHR_BASE_URL must be an https origin (plain http only to this machine)".into(),
        ))
    }
}

fn command(program: &Path, args: &[OsString], api: &str, channel: &str) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(program);
    c.args(args)
        .env("IOHR_EXT_API", api)
        .env("IOHR_EXT_TOKEN_SOCKET", channel)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    // Credentials stay with iohr: the extension asks the channel instead.
    for (k, _) in std::env::vars_os() {
        if k.to_str()
            .is_some_and(|k| k == "IOHR_TOKEN" || k.starts_with("IOHR_TOKEN_"))
        {
            c.env_remove(k);
        }
    }
    c
}

fn exit_code(status: std::process::ExitStatus) -> u8 {
    if let Some(code) = status.code() {
        return u8::try_from(code & 0xff).unwrap_or(1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        if let Some(sig) = status.signal() {
            return u8::try_from(128 + (sig & 0x7f)).unwrap_or(1);
        }
    }
    1
}

/// A private directory that is removed when dropped.
#[cfg(unix)]
struct Private(std::path::PathBuf);

#[cfg(unix)]
impl Drop for Private {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn spawn_failed(program: &Path, e: &std::io::Error) -> Error {
    Error::Failed(format!("cannot start {}: {e}", program.display()))
}

#[cfg(unix)]
async fn serve<M: Minter>(
    program: &Path,
    api: &str,
    args: &[OsString],
    ctx: Arc<Ctx<M>>,
) -> Result<u8, Error> {
    use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, PermissionsExt as _};

    let dir = std::env::temp_dir().join(format!("iohr-ext-{:016x}", getrandom::u64().unwrap_or(0)));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(|e| Error::Failed(format!("cannot create {}: {e}", dir.display())))?;
    let guard = Private(dir.clone());
    let me = std::fs::metadata(&dir)
        .map_err(|e| Error::Failed(format!("cannot read {}: {e}", dir.display())))?
        .uid();
    let path = dir.join("token.sock");
    let listener = tokio::net::UnixListener::bind(&path)
        .map_err(|e| Error::Failed(format!("cannot open the token channel: {e}")))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| Error::Failed(format!("cannot restrict the token channel: {e}")))?;
    let mut child = command(program, args, api, &path.to_string_lossy())
        .spawn()
        .map_err(|e| spawn_failed(program, &e))?;
    let status = loop {
        tokio::select! {
            status = child.wait() => break status,
            accepted = listener.accept() => {
                if let Ok((conn, _)) = accepted {
                    // Only this user's processes: the directory is 0700, and the peer
                    // is checked as well.
                    if conn.peer_cred().is_ok_and(|c| c.uid() == me) {
                        tokio::spawn(handle(conn, Arc::clone(&ctx)));
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                // The extension got the interrupt too; wait for it to finish.
            }
        }
    };
    drop(listener);
    drop(guard);
    let status = status.map_err(|e| Error::Failed(format!("lost the extension: {e}")))?;
    Ok(exit_code(status))
}

#[cfg(windows)]
async fn serve<M: Minter>(
    program: &Path,
    api: &str,
    args: &[OsString],
    ctx: Arc<Ctx<M>>,
) -> Result<u8, Error> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let name = format!(
        r"\\.\pipe\iohr-ext-{:016x}{:016x}",
        getrandom::u64().unwrap_or(0),
        getrandom::u64().unwrap_or(0)
    );
    let open = |first: bool| {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create(&name)
            .map_err(|e| Error::Failed(format!("cannot open the token channel: {e}")))
    };
    let mut server = open(true)?;
    let mut child = command(program, args, api, &name)
        .spawn()
        .map_err(|e| spawn_failed(program, &e))?;
    let status = loop {
        tokio::select! {
            status = child.wait() => break status,
            connected = server.connect() => {
                if connected.is_ok() {
                    let conn = std::mem::replace(&mut server, open(false)?);
                    tokio::spawn(handle(conn, Arc::clone(&ctx)));
                }
            }
            _ = tokio::signal::ctrl_c() => {}
        }
    };
    drop(server);
    let status = status.map_err(|e| Error::Failed(format!("lost the extension: {e}")))?;
    Ok(exit_code(status))
}
