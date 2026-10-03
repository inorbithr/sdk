use std::path::PathBuf;
use std::sync::Arc;

use iohr_auth::{
    AuthError, Bearer, Claims, Config, Credential, EntryKey, FileStore, KeyringStore, Kind,
    ProfileName, Redacted, Session as PersonSession, StaticToken, Storage, Store, StoreError,
};
use time::OffsetDateTime;

use crate::Env;
use crate::api::{Api, base_url};
use crate::cli::Global;
use crate::env_name;
use crate::error::Error;

/// The config directory and the config read from it.
pub(crate) struct Ctx {
    pub(crate) dir: PathBuf,
    pub(crate) config: Config,
}

impl Ctx {
    pub(crate) fn load(global: &Global) -> Result<Self, Error> {
        let dir = match &global.config_dir {
            Some(d) => d.clone(),
            None => Config::default_dir()?,
        };
        let config = Config::load(&dir)?;
        Ok(Self { dir, config })
    }

    pub(crate) fn save(&self) -> Result<(), Error> {
        Ok(self.config.save(&self.dir)?)
    }

    /// The store a profile's secret lives in.
    pub(crate) fn store(&self, storage: Storage) -> Result<Arc<dyn Store>, Error> {
        Ok(match storage {
            Storage::File => Arc::new(FileStore::new(self.dir.join("secrets"))),
            _ => Arc::new(KeyringStore::open()?),
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
    tokio::task::spawn_blocking(move || f(store.as_ref()))
        .await
        .map_err(|e| Error::Failed(format!("the credential store call stopped: {e}")))?
        .map_err(Error::from)
}

/// The credential a run calls with: an API token, or a person's refreshing session.
#[derive(Debug)]
pub(crate) enum AnyCredential {
    Token(StaticToken),
    Person(Box<PersonSession>),
}

impl Credential for AnyCredential {
    async fn bearer(&self) -> Result<Bearer, AuthError> {
        match self {
            Self::Token(t) => t.bearer().await,
            Self::Person(p) => p.bearer().await,
        }
    }
}

/// Who calls, against which account, with which client.
pub(crate) struct Session {
    /// The profile's name, or `IOHR_TOKEN`.
    pub(crate) label: String,
    pub(crate) kind: Kind,
    pub(crate) account: String,
    pub(crate) claims: Claims,
    pub(crate) api: Api<AnyCredential>,
}

/// The session for this run: `IOHR_TOKEN` when set, otherwise the chosen profile.
pub(crate) async fn session(global: &Global, env: &Env) -> Result<Session, Error> {
    let ctx = Ctx::load(global)?;
    let chosen = ctx.chosen(global).ok();
    session_for(global, env, &ctx, chosen.as_ref()).await
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
    let base = base_url(&global.base_url)?;
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
        let api = Api::new(
            base,
            AnyCredential::Token(StaticToken::new(token.clone())),
            global.verbose,
        )?;
        return Ok(Session {
            label: name.map_or_else(
                || "IOHR_TOKEN".into(),
                |n| format!("{n} (IOHR_TOKEN_{})", env_name(n.as_str())),
            ),
            kind: Kind::Token,
            account,
            claims,
            api,
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
    let store = ctx.store(profile.storage)?;
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
            let issuer = profile.issuer.as_deref().unwrap_or(&global.issuer);
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
    let api = Api::new(base, credential, global.verbose)?;
    Ok(Session {
        label: name.to_string(),
        kind: profile.kind,
        account: profile.account,
        claims,
        api,
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
