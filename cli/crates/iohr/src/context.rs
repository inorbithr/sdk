use std::path::PathBuf;
use std::sync::Arc;

use inorbithr::{
    Attempt, Client, ClientCredentials, ConfigError, Hook, Method, Operation, RawResponse, Secret,
    Token, TokenProvider,
};
use iohr_auth::{
    AuthError, Bearer, Claims, Config, Credential, EntryKey, FileStore, KeyringStore, Kind,
    ProfileName, QuietStore, Redacted, Session as PersonSession, StaticToken, Storage, Store,
    StoreError,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use time::OffsetDateTime;

use crate::Env;
use crate::cli::Global;
use crate::env_name;
use crate::error::Error;

/// Whether a credential-store call may wait on, or show, a prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Prompts {
    /// The Keychain may ask the person; the call waits for the answer.
    Allowed,
    /// The call never waits and never shows a prompt: where one would show, it fails with
    /// [`Error::StoreWouldAsk`].
    Never,
}

#[cfg(test)]
thread_local! {
    /// How many times this thread opened a credential store (the tests count them).
    static OPENED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The config directory and the config read from it.
pub(crate) struct Ctx {
    pub(crate) dir: PathBuf,
    pub(crate) config: Config,
}

/// The config directory: `--config-dir`/`IOHR_CONFIG_DIR`, else the platform's. The one place
/// that decides it (profiles, the user-level `IO.md`).
pub(crate) fn config_dir(global: &Global) -> Result<PathBuf, Error> {
    match &global.config_dir {
        Some(d) => Ok(d.clone()),
        None => Ok(Config::default_dir()?),
    }
}

impl Ctx {
    pub(crate) fn load(global: &Global) -> Result<Self, Error> {
        let dir = config_dir(global)?;
        let config = Config::load(&dir)?;
        Ok(Self { dir, config })
    }

    pub(crate) fn save(&self) -> Result<(), Error> {
        Ok(self.config.save(&self.dir)?)
    }

    /// The store a profile's secret lives in. `prompts` says whether a call may wait on, or
    /// show, a credential prompt: [`Prompts::Never`] wraps the OS store in a [`QuietStore`]
    /// (the shell prompt's paths); the file store never prompts and is never wrapped.
    pub(crate) fn store(
        &self,
        storage: Storage,
        prompts: Prompts,
    ) -> Result<Arc<dyn Store>, Error> {
        #[cfg(test)]
        OPENED.with(|n| n.set(n.get() + 1));
        Ok(match (storage, prompts) {
            (Storage::File, _) => Arc::new(FileStore::new(self.dir.join("secrets"))),
            (_, Prompts::Allowed) => Arc::new(KeyringStore::open()?),
            (_, Prompts::Never) => Arc::new(QuietStore::new(Arc::new(KeyringStore::open()?))),
        })
    }

    /// The profile chosen by `--profile`, `IOHR_PROFILE`, or the default.
    pub(crate) fn chosen(&self, global: &Global) -> Result<ProfileName, Error> {
        global
            .profile
            .clone()
            .or_else(|| self.config.default.clone())
            .ok_or_else(|| {
                Error::NotSignedIn(
                    "no profile on this machine: create an API token in the console, then run \
                 `iohr login --with-token < token.txt`, or set IOHR_TOKEN"
                        .into(),
                )
            })
    }
}

/// Runs a credential-store call on a blocking thread.
pub(crate) async fn blocking<T, F>(store: Arc<dyn Store>, f: F) -> Result<T, Error>
where
    T: Send + 'static,
    F: FnOnce(&dyn Store) -> Result<T, StoreError> + Send + 'static,
{
    iohr_auth::off_thread(move || f(store.as_ref()))
        .await
        .map_err(Error::from)
}

/// The credential a run calls with: an API token, or a person's refreshing session.
#[derive(Debug)]
pub(crate) enum AnyCredential {
    Token(StaticToken),
    Person(Box<PersonSession>),
    /// Io's own sign-in (`iohr-io`, client credentials): minted on first use and again
    /// before each token ends, held in memory only (RFC 0065.2).
    Io(Box<ClientCredentials>),
}

impl Credential for AnyCredential {
    async fn bearer(&self) -> Result<Bearer, AuthError> {
        match self {
            Self::Token(t) => t.bearer().await,
            Self::Person(p) => p.bearer().await,
            Self::Io(c) => {
                let token = c
                    .token()
                    .await
                    .map_err(|e| AuthError::Http(format!("Io's sign-in failed: {e}")))?;
                Ok(Bearer::new(Redacted::new(token.expose().to_owned())))
            }
        }
    }
}

/// The command line's credential is the runtime's token provider (ADR 0009): a
/// person's session refreshes on its own, so the runtime is handed a token that is
/// valid now and never caches it.
impl TokenProvider for AnyCredential {
    async fn invalidate(&self) {
        // Io's cached token goes when the API refuses it, so the next call mints again.
        if let Self::Io(c) = self {
            c.invalidate().await;
        }
    }

    async fn token(&self) -> Result<Token, inorbithr::AuthError> {
        let bearer = self
            .bearer()
            .await
            .map_err(|e| inorbithr::AuthError::Provider(e.to_string()))?;
        Ok(Token::new(Secret::from(bearer.expose()), None))
    }
}

/// The credential shared between the runtime's client and the extension token channel
/// (ADR 0012), which hands the same access token to an extension.
#[derive(Debug, Clone)]
pub(crate) struct Shared(pub(crate) Arc<AnyCredential>);

impl TokenProvider for Shared {
    /// The client calls this once after a 401, so the retry carries a fresh token
    /// (without it Io's cached token, stale after a sleep, was sent again for ever).
    async fn invalidate(&self) {
        self.0.invalidate().await;
    }

    async fn token(&self) -> Result<Token, inorbithr::AuthError> {
        self.0.token().await
    }
}

/// `--verbose`: one line per attempt on stderr with method, path, status, time and
/// request id; never a header, a query value or a body (SR-13).
struct Verbose {
    started: std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
}

impl Verbose {
    fn new() -> Self {
        Self {
            started: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn line(&self, attempt: &Attempt, status: &str) {
        let elapsed = self
            .started
            .lock()
            .ok()
            .and_then(|mut m| m.remove(&attempt.request_id))
            .map_or(0, |t| t.elapsed().as_millis());
        note(&format!(
            "{} {} -> {status} in {elapsed} ms (request id {}, attempt {})",
            attempt.method, attempt.path, attempt.request_id, attempt.number
        ));
    }
}

impl Hook for Verbose {
    fn on_request(&self, attempt: &Attempt) {
        if let Ok(mut m) = self.started.lock() {
            m.insert(attempt.request_id.clone(), std::time::Instant::now());
        }
    }

    fn on_response(&self, attempt: &Attempt, response: &RawResponse) {
        self.line(attempt, &response.status.to_string());
    }

    fn on_error(&self, attempt: &Attempt, error: &inorbithr::Error) {
        if error.status().is_none() {
            self.line(attempt, "no answer");
        }
    }
}

/// One line on stderr, for `--verbose`.
pub(crate) fn note(line: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr().lock(), "iohr: {line}");
}

/// A successful answer, as the commands read it.
#[derive(Debug)]
pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) content_type: Option<String>,
    pub(crate) request_id: Option<String>,
    pub(crate) body: Vec<u8>,
}

/// Sets `x-iohr-via` on every call: attribution only (RFC 0065.2).
struct ViaMw(String);

impl inorbithr::middleware::Middleware for ViaMw {
    fn name(&self) -> &'static str {
        "iohr_via"
    }

    fn handle<'a>(
        &'a self,
        mut req: inorbithr::middleware::Request,
        next: inorbithr::middleware::Next<'a>,
    ) -> inorbithr::middleware::BoxFuture<
        'a,
        Result<inorbithr::middleware::Response, inorbithr::Error>,
    > {
        req.headers_mut().insert("x-iohr-via", self.0.clone());
        Box::pin(next.run(req))
    }
}

/// The `x-iohr-via` value the `--via-*` flags name, in the order `via.rs` reads, or `None`
/// when none is set. Each value is 1 to 64 of `[A-Za-z0-9._:-]`: the platform drops
/// anything else, so a value that would be dropped is refused here instead.
pub(crate) fn via_header(global: &Global) -> Result<Option<String>, Error> {
    let mut parts = Vec::new();
    for (key, flag, v) in [
        ("personality", "--via-role", &global.via_role().cloned()),
        ("session", "--via-session", &global.via_session),
        ("job", "--via-job", &global.via_job),
    ] {
        let Some(v) = v.as_deref().map(str::trim) else {
            continue;
        };
        if v.is_empty()
            || v.len() > 64
            || !v
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
        {
            return Err(Error::Usage(format!(
                "{flag} is 1 to 64 letters, digits, '.', '_', ':' or '-'"
            )));
        }
        parts.push(format!("{key}={v}"));
    }
    Ok((!parts.is_empty()).then(|| parts.join("; ")))
}

/// The calls every command makes, on the runtime's client.
pub(crate) struct Api {
    client: Client,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api")
            .field("base", &self.client.base_url())
            .finish_non_exhaustive()
    }
}

impl Api {
    pub(crate) fn new(
        base: &str,
        credential: Arc<AnyCredential>,
        verbose: bool,
    ) -> Result<Self, Error> {
        Self::with_via(base, credential, verbose, None)
    }

    /// [`new`](Self::new) that also sets `x-iohr-via` (RFC 0065.2) on every call.
    pub(crate) fn with_via(
        base: &str,
        credential: Arc<AnyCredential>,
        verbose: bool,
        via: Option<String>,
    ) -> Result<Self, Error> {
        Self::with_streams(
            base,
            credential,
            verbose,
            via,
            inorbithr::Streams::default(),
        )
    }

    /// [`with_via`](Self::with_via) that also chooses how streams open: `Streams::Socket`
    /// carries all of this client's streams over one `/v1/ws` connection (the page's live
    /// socket, `iohr ui serve`).
    pub(crate) fn with_streams(
        base: &str,
        credential: Arc<AnyCredential>,
        verbose: bool,
        via: Option<String>,
        streams: inorbithr::Streams,
    ) -> Result<Self, Error> {
        let mut b = Client::builder()
            .streams(streams)
            .base_url(base)
            .token_provider(Shared(credential))
            .user_agent_suffix(concat!("iohr/", env!("CARGO_PKG_VERSION")));
        if verbose {
            b = b.hook(Verbose::new());
        }
        if let Some(value) = via {
            b = b.pipeline(move |p| p.add_per_call(ViaMw(value)));
        }
        // A base URL that cannot be used is a failed call (exit 1), as it always was,
        // named by the variable that set it.
        let client = b.build().map_err(|e| match e {
            inorbithr::Error::Config(ConfigError::InvalidUrl { .. }) => {
                Error::Failed(format!("IOHR_BASE_URL: {e}"))
            }
            other => Error::Failed(other.to_string()),
        })?;
        Ok(Self { client })
    }

    /// Another handle on the same client (it shares the connection pool and credential),
    /// for work that outlives the call, such as a detached key revoke.
    pub(crate) fn handle(&self) -> Self {
        Self {
            client: self.client.clone(),
        }
    }

    /// The runtime's client, for the generated surface (`inorbithr::public`).
    pub(crate) fn client(&self) -> &Client {
        &self.client
    }

    /// `GET path` and read the JSON answer.
    pub(crate) async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, inorbithr::Error> {
        let raw = self.send(Method::Get, path, query, None).await?;
        raw.json()
    }

    /// [`send`](Self::send) with any serialisable body, so a body that carries a secret
    /// borrows it instead of copying it into a [`Value`]; the bytes sent are the
    /// runtime's, dropped with the call.
    pub(crate) async fn send_body<B: serde::Serialize + ?Sized>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> Result<RawResponse, inorbithr::Error> {
        let op = Operation::new(method, path.to_owned()).json(body)?;
        self.client.send(op).await
    }

    /// One call, as the runtime makes it: idempotent methods retried, a 401 answered by
    /// one fresh token (SR-18).
    pub(crate) async fn send(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<RawResponse, inorbithr::Error> {
        let mut op = Operation::new(method, path.to_owned());
        for (k, v) in query {
            op = op.query((*k).to_owned(), (*v).to_owned());
        }
        if let Some(b) = body {
            op = op.json(b)?;
        }
        self.client.send(op).await
    }
}

/// The answer of [`Api::send`] in the shape the `api` command prints.
pub(crate) fn response(raw: RawResponse) -> Response {
    Response {
        status: raw.status,
        content_type: raw.headers.get("content-type").map(str::to_owned),
        request_id: raw.server_request_id.clone(),
        body: raw.body,
    }
}

/// Who calls, against which account, with which client.
pub(crate) struct Session {
    /// The profile's name, or `IOHR_TOKEN`.
    pub(crate) label: String,
    /// The profile, when the run has one (none for `IOHR_TOKEN`).
    pub(crate) profile: Option<ProfileName>,
    pub(crate) kind: Kind,
    pub(crate) account: String,
    pub(crate) claims: Claims,
    pub(crate) api: Api,
    /// The credential `api` calls with, for the extension token channel.
    pub(crate) credential: Arc<AnyCredential>,
}

/// The session for this run: a profile named by `--profile` or `IOHR_PROFILE`, else
/// `IOHR_TOKEN` when set, else the default profile. `IOHR_TOKEN` wins over the default
/// profile (README, ADR 0009): a token set for one command is the one it calls with.
pub(crate) async fn session(global: &Global, env: &Env) -> Result<Session, Error> {
    let ctx = Ctx::load(global)?;
    let chosen = run_profile(global, env, &ctx);
    session_for(global, env, &ctx, chosen.as_ref()).await
}

/// [`session`] for the shell prompt's paths: the credential store never shows a prompt and
/// never makes the call wait; where it would, the error is [`Error::StoreWouldAsk`].
pub(crate) async fn quiet_session(global: &Global, env: &Env) -> Result<Session, Error> {
    let ctx = Ctx::load(global)?;
    let chosen = run_profile(global, env, &ctx);
    session_in(global, env, &ctx, chosen.as_ref(), true).await
}

type Slot = Arc<tokio::sync::OnceCell<Arc<Session>>>;

/// The sessions of this process, by what makes one: the config directory, the profile, the
/// API address and the tokens in the environment (as a hash, never the token).
static SHARED: std::sync::Mutex<std::collections::BTreeMap<String, Slot>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

/// [`session`] for a run that asks for it more than once (`iohr project init` checks the
/// sign-in, the space and the plan, one after the other): the first ask loads the
/// credential and every later one in this process gets the same session, so the credential
/// store is read once. On a Mac, where each read can be a Keychain dialog, that is the
/// difference between one prompt and six. Only a session is kept, never a refusal: a
/// refused ask (a Keychain dialog answered No, a sign-in made since) can succeed on the
/// next one. Never for a process that outlives its sign-in (a runner, the panel): they
/// call [`session`] and see a sign-out as it happens.
///
/// # Errors
///
/// What [`session`] returns.
pub(crate) async fn shared_session(global: &Global, env: &Env) -> Result<Arc<Session>, Error> {
    use std::hash::{Hash as _, Hasher as _};
    let ctx = Ctx::load(global)?;
    let chosen = run_profile(global, env, &ctx);
    let secret = |t: Option<&Redacted<String>>| {
        t.map(|t| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            t.expose().hash(&mut h);
            h.finish()
        })
    };
    let key = format!(
        "{}|{}|{}|{:?}|{:?}",
        ctx.dir.display(),
        chosen.as_ref().map_or("", ProfileName::as_str),
        global.base_url(),
        secret(env.token.as_ref()),
        secret(chosen.as_ref().and_then(|n| env.profile_token(n.as_str()))),
    );
    let slot = SHARED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entry(key)
        .or_default()
        .clone();
    slot.get_or_try_init(|| async {
        session_for(global, env, &ctx, chosen.as_ref())
            .await
            .map(Arc::new)
    })
    .await
    .cloned()
}

/// The profile a run uses: `--profile` or `IOHR_PROFILE`; none when `IOHR_TOKEN` is set
/// without one; otherwise the default profile, if any.
pub(crate) fn run_profile(global: &Global, env: &Env, ctx: &Ctx) -> Option<ProfileName> {
    match &global.profile {
        Some(name) => Some(name.clone()),
        None if env.token.is_some() => None,
        None => ctx.config.default.clone(),
    }
}

/// The session of one named profile: `IOHR_TOKEN_<PROFILE>` when set (CI, where a
/// person cannot sign in), otherwise the profile's own credential. With `name` absent,
/// `IOHR_TOKEN` alone.
pub(crate) async fn session_for(
    global: &Global,
    env: &Env,
    ctx: &Ctx,
    name: Option<&ProfileName>,
) -> Result<Session, Error> {
    session_in(global, env, ctx, name, false).await
}

/// `quiet`: the OS store is read through a [`QuietStore`] ([`Prompts::Never`]).
async fn session_in(
    global: &Global,
    env: &Env,
    ctx: &Ctx,
    name: Option<&ProfileName>,
    quiet: bool,
) -> Result<Session, Error> {
    let base = global.base_url();
    // A named profile takes `IOHR_TOKEN_<PROFILE>` and nothing else from the environment;
    // the chosen profile falls back to `IOHR_TOKEN` as every command always did.
    let per_profile: Option<&Redacted<String>> = name.and_then(|n| env.profile_token(n.as_str()));
    let standing_in = per_profile.or(if name.is_some() {
        None
    } else {
        env.token.as_ref()
    });
    if let Some(token) = standing_in {
        let claims = Claims::read(token.expose(), OffsetDateTime::now_utc())?;
        let account = claims.org.clone().unwrap_or_default();
        let credential = Arc::new(AnyCredential::Token(StaticToken::new(token.clone())));
        let api = Api::new(base, Arc::clone(&credential), global.verbose)?;
        return Ok(Session {
            label: name.map_or_else(
                || "IOHR_TOKEN".into(),
                |n| format!("{n} (IOHR_TOKEN_{})", env_name(n.as_str())),
            ),
            profile: name.cloned(),
            kind: Kind::Token,
            account,
            claims,
            api,
            credential,
        });
    }
    let Some(name) = name else {
        return Err(Error::NotSignedIn(
            "no profile on this machine: create an API token in the console, then run \
             `iohr login --with-token < token.txt`, or set IOHR_TOKEN"
                .into(),
        ));
    };
    let name = name.clone();
    let profile = ctx.config.profile(&name).cloned().ok_or_else(|| {
        Error::NotSignedIn(format!(
            "there is no profile {name}: `iohr profile list` shows the ones there are"
        ))
    })?;
    let key = EntryKey::new(name.clone(), &profile.account)?;
    let store = ctx.store(
        profile.storage,
        if quiet {
            Prompts::Never
        } else {
            Prompts::Allowed
        },
    )?;
    let (credential, claims) = match profile.kind {
        Kind::Token => {
            let secret: Option<Redacted<String>> = blocking(store, move |s| s.get(&key)).await?;
            let token = secret.ok_or_else(|| AuthError::NotSignedIn {
                profile: name.to_string(),
            })?;
            let claims = Claims::read(token.expose(), OffsetDateTime::now_utc()).map_err(|e| {
                Error::NotSignedIn(format!(
                    "profile {name}: {e}; then `iohr login --with-token --profile {name}`"
                ))
            })?;
            (AnyCredential::Token(StaticToken::new(token)), claims)
        }
        Kind::Person => {
            let issuer = profile.issuer.as_deref().unwrap_or(global.issuer());
            let client_id = profile.client_id.as_deref().unwrap_or(&global.client_id);
            let person = PersonSession::load(store, key, issuer, client_id).await?;
            // Refreshes now if needed, so the claims shown are the ones calls carry.
            let bearer = person.bearer().await?;
            let claims = Claims::read(bearer.expose(), OffsetDateTime::now_utc())?;
            (AnyCredential::Person(Box::new(person)), claims)
        }
        _ => {
            return Err(Error::Failed(format!(
                "profile {name} was made by a newer iohr; update iohr to use it"
            )));
        }
    };
    let credential = Arc::new(credential);
    let api = Api::new(base, Arc::clone(&credential), global.verbose)?;
    Ok(Session {
        label: name.to_string(),
        profile: Some(name),
        kind: profile.kind,
        account: profile.account,
        claims,
        api,
        credential,
    })
}

/// The session of a runner that signs in as Io (RFC 0065.2): the client credentials grant
/// for `iohr-io`, minted on first use, held in memory, minted again before it ends.
///
/// The profile (`--profile`, else the default) still names where the runner's keys and
/// settings live; only the credential differs. The account is the token's `org`, so a
/// runner for a team passes `--account`.
pub(crate) async fn io_session(global: &Global, ctx: &Ctx) -> Result<Session, Error> {
    use crate::io_signin::{self, SecretSource};
    let via = via_header(global)?;
    let source = SecretSource::choose(std::env::var(io_signin::SECRET_FILE_VAR).ok().as_deref())?;
    let secret = source.read().await?;
    let provider = io_signin::credential(global.issuer(), secret)?;
    let credential = Arc::new(AnyCredential::Io(Box::new(provider)));
    // The first mint, so a wrong secret or an unreachable sign-in service stops here.
    let bearer = credential
        .bearer()
        .await
        .map_err(|e| Error::NotSignedIn(e.to_string()))?;
    let claims = Claims::read(bearer.expose(), OffsetDateTime::now_utc())
        .map_err(|e| Error::NotSignedIn(format!("Io's token is not usable: {e}")))?;
    let api = Api::with_via(
        global.base_url(),
        Arc::clone(&credential),
        global.verbose,
        via,
    )?;
    Ok(Session {
        label: format!("Io ({})", io_signin::IO_CLIENT_ID),
        profile: global
            .profile
            .clone()
            .or_else(|| ctx.config.default.clone()),
        kind: Kind::Token,
        account: claims.org.clone().unwrap_or_default(),
        claims,
        api,
        credential,
    })
}

/// The account a command acts on: `--account`, else the session's.
pub(crate) fn account(session: &Session, flag: Option<String>) -> Result<String, Error> {
    let a = flag.unwrap_or_else(|| session.account.clone());
    if a.is_empty() {
        return Err(Error::Usage(
            "this credential names no account; pass --account".into(),
        ));
    }
    if !a
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(Error::Usage(
            "an account id has only letters, digits, '-' and '_'".into(),
        ));
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use base64::Engine as _;
    use clap::Parser as _;
    use iohr_auth::{Config, EntryKey, FileStore, Kind, Profile, Redacted, Storage, Store as _};

    use super::{OPENED, shared_session};
    use crate::Env;
    use crate::cli::Cli;
    use crate::error::Error;

    /// Puts the profile `work`'s API token in the file store under `dir`.
    fn sign_in(dir: &std::path::Path) {
        let b64 = |v: serde_json::Value| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string())
        };
        let jwt = format!(
            "{}.{}.sig",
            b64(serde_json::json!({"alg": "none"})),
            b64(serde_json::json!({"sub": "ak_x", "aud": ["iohr-api"], "org": "acc_1"}))
        );
        let key = EntryKey::new("work".parse().unwrap(), "acc_1").unwrap();
        FileStore::new(dir.join("secrets"))
            .set(&key, &Redacted::new(jwt))
            .unwrap();
    }

    /// A config directory with the profile `work` holding an API token in the file store,
    /// when `token` is true; the global flags that point at it.
    fn machine(token: bool) -> (tempfile::TempDir, crate::cli::Global) {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        let name: iohr_auth::ProfileName = "work".parse().unwrap();
        config.profiles.insert(
            name.clone(),
            Profile::new(Kind::Token, "acc_1", Storage::File),
        );
        config.default = Some(name.clone());
        config.save(dir.path()).unwrap();
        if token {
            sign_in(dir.path());
        }
        let cli = Cli::try_parse_from([
            "iohr",
            "--config-dir",
            dir.path().to_str().unwrap(),
            "whoami",
        ])
        .unwrap();
        (dir, cli.global)
    }

    fn opened() -> usize {
        OPENED.with(std::cell::Cell::get)
    }

    /// Project init asks for the session six times; the credential store must be opened
    /// (on a Mac: the Keychain read) once, whatever the number of asks.
    #[tokio::test]
    async fn one_credential_session_serves_every_ask_in_a_run() {
        let (_dir, g) = machine(true);
        let env = Env::default();
        let before = opened();
        for _ in 0..3 {
            let s = shared_session(&g, &env).await.unwrap();
            assert_eq!(s.label, "work");
        }
        assert_eq!(opened() - before, 1, "the store was opened again");
    }

    /// A refusal is not remembered: the ask that failed (no sign-in yet) does not decide the
    /// next one, which finds the credential once it is there.
    #[tokio::test]
    async fn a_refused_session_is_not_cached_so_a_later_ask_can_succeed() {
        let (dir, g) = machine(false);
        let env = Env::default();
        let before = opened();
        let e = shared_session(&g, &env).await.err().unwrap();
        assert!(matches!(e, Error::NotSignedIn(_)), "{e:?}");
        // The person signs in between the two asks.
        sign_in(dir.path());
        let s = shared_session(&g, &env).await.unwrap();
        assert_eq!(s.label, "work");
        assert_eq!(
            opened() - before,
            2,
            "the second ask must read the store again"
        );
    }
}
