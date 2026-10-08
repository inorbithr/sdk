use std::io::{IsTerminal as _, Read as _};

use std::fmt::Write as _;
use std::sync::Arc;

use iohr_auth::{
    AuthError, Authorization, Browser, Claims, Device, EntryKey, Kind, Profile, ProfileName,
    Provider, Redacted, Session, Storage,
};
use time::OffsetDateTime;
use zeroize::Zeroizing;

use crate::Env;
use crate::browser;
use crate::cli::{Global, Login};
use crate::context::{Ctx, blocking};
use crate::error::Error;
use crate::output::Out;

/// The profile a first `login` creates when no `--profile` is given.
const DEFAULT_PROFILE: &str = "default";

/// The largest token read from standard input.
const MAX_TOKEN: u64 = 64 * 1024;

pub(crate) async fn login(g: &Global, env: &Env, args: &Login, out: Out) -> Result<(), Error> {
    if env.token.is_some() {
        Out::note("note: IOHR_TOKEN is set, and it wins over profiles until it is unset");
    }
    if args.how.with_token {
        with_token(g, args, out).await
    } else {
        person(g, args, out).await
    }
}

async fn with_token(g: &Global, args: &Login, out: Out) -> Result<(), Error> {
    let token = read_token()?;
    let claims = Claims::read(token.expose(), OffsetDateTime::now_utc())
        .map_err(|e| Error::Usage(e.to_string()))?;
    let account = claims
        .org
        .clone()
        .ok_or_else(|| Error::Usage("this token names no account (no org claim)".into()))?;
    let mut ctx = Ctx::load(g)?;
    let (name, storage) = target(&ctx, g, args)?;
    let key = replace_secret(&ctx, &name, &account, storage).await?;
    blocking(ctx.store(storage)?, move |s| s.set(&key, &token)).await?;
    remember(
        &mut ctx,
        &name,
        Profile::new(Kind::Token, account.clone(), storage),
    )?;

    let expires = claims
        .expires_at()
        .map_or_else(|| "never".into(), |t| t.date().to_string());
    if out.json {
        Out::print_json(&serde_json::json!({
            "profile": name.as_str(), "kind": "token", "account": account,
            "scopes": claims.scp, "expires_at": expires, "storage": storage_name(storage),
        }));
    } else {
        Out::note(&format!(
            "Signed in as profile {name}: account {account}, {} scope(s), expires {expires}, kept in {}.",
            claims.scp.len(),
            storage_name(storage)
        ));
    }
    Ok(())
}

/// A person signs in: in a browser on this machine when one can be opened (or with
/// `--web`), with a device code otherwise (or with `--device`).
async fn person(g: &Global, args: &Login, out: Out) -> Result<(), Error> {
    let mut ctx = Ctx::load(g)?;
    let (name, storage) = target(&ctx, g, args)?;
    let provider = Provider::discover(&g.issuer, &g.client_id).await?;
    let use_device = args.how.device || (!args.how.web && !browser::available());
    let granted = if use_device {
        let auth = Authorization::<Device>::start(provider).await?;
        let mut text = format!(
            "To sign in, open {} in any browser and enter this code:\n\n    {}\n",
            auth.verification_uri(),
            auth.user_code()
        );
        if let Some(full) = auth.verification_uri_complete() {
            let _ = write!(text, "\nOr open {full}\n");
        }
        text.push_str("\nOnly enter a code you started yourself. Waiting for approval...");
        Out::note(&text);
        auth.finish().await?
    } else {
        let auth = Authorization::<Browser>::start(provider)
            .await?
            .for_profile(name.as_str());
        let url = auth.url().to_string();
        if browser::open(&url) {
            Out::note(&format!(
                "Opened your browser to sign in. If it did not open, visit:\n\n    {url}\n"
            ));
        } else {
            Out::note(&format!(
                "Open this link in a browser on this machine to sign in:\n\n    {url}\n"
            ));
        }
        Out::note("Waiting for the browser (at most 5 minutes)...");
        auth.finish().await?
    };
    let account = granted.account().map(str::to_owned).ok_or_else(|| {
        Error::Failed("the sign-in named no account; open the console once, then try again".into())
    })?;
    let subject = granted.subject().to_owned();
    let key = replace_secret(&ctx, &name, &account, storage).await?;
    Session::create(granted, ctx.store(storage)?, key).await?;
    remember(
        &mut ctx,
        &name,
        Profile::person(
            account.clone(),
            storage,
            g.issuer.clone(),
            g.client_id.clone(),
        ),
    )?;
    if out.json {
        Out::print_json(&serde_json::json!({
            "profile": name.as_str(), "kind": "person", "subject": subject, "account": account,
            "storage": storage_name(storage),
        }));
    } else {
        Out::note(&format!(
            "Signed in as profile {name}: account {account}, kept in {}.",
            storage_name(storage)
        ));
    }
    Ok(())
}

/// The profile to write and where its secret goes.
fn target(ctx: &Ctx, g: &Global, args: &Login) -> Result<(ProfileName, Storage), Error> {
    let name = match (&g.profile, &ctx.config.default) {
        (Some(n), _) | (None, Some(n)) => n.clone(),
        (None, None) => DEFAULT_PROFILE
            .parse::<ProfileName>()
            .map_err(|e| Error::Failed(e.to_string()))?,
    };
    Ok((
        name,
        if args.insecure_storage {
            Storage::File
        } else {
            Storage::Keyring
        },
    ))
}

/// A profile that pointed at another account or store loses its old secret; the
/// entry for the new one is returned.
async fn replace_secret(
    ctx: &Ctx,
    name: &ProfileName,
    account: &str,
    storage: Storage,
) -> Result<EntryKey, Error> {
    if let Some(old) = ctx.config.profile(name).cloned()
        && (old.account != account || old.storage != storage)
    {
        let old_key = EntryKey::new(name.clone(), &old.account)?;
        blocking(ctx.store(old.storage)?, move |s| s.delete(&old_key)).await?;
    }
    Ok(EntryKey::new(name.clone(), account)?)
}

fn remember(ctx: &mut Ctx, name: &ProfileName, profile: Profile) -> Result<(), Error> {
    ctx.config.profiles.insert(name.clone(), profile);
    if ctx.config.default.is_none() {
        ctx.config.default = Some(name.clone());
    }
    ctx.save()
}

pub(crate) async fn logout(g: &Global, out: Out) -> Result<(), Error> {
    let mut ctx = Ctx::load(g)?;
    let name = ctx.chosen(g)?;
    let profile = ctx
        .config
        .profile(&name)
        .cloned()
        .ok_or_else(|| Error::Usage(format!("there is no profile {name}")))?;
    let key = EntryKey::new(name.clone(), &profile.account)?;
    let store = ctx.store(profile.storage)?;
    // A person's session is ended at the sign-in service first; if that fails,
    // nothing is removed, so the next try can still revoke.
    let mut revoked = false;
    if profile.kind == Kind::Person {
        let issuer = profile.issuer.as_deref().unwrap_or(&g.issuer);
        let client_id = profile.client_id.as_deref().unwrap_or(&g.client_id);
        match Session::load(Arc::clone(&store), key.clone(), issuer, client_id).await {
            Ok(session) => {
                session.revoke().await.map_err(|e| {
                    Error::Failed(format!(
                        "{e}. Nothing was removed: try again, or sign this device out in the console."
                    ))
                })?;
                revoked = true;
            }
            Err(AuthError::NotSignedIn { .. }) => {}
            Err(e) => return Err(e.into()),
        }
    }
    let removed = blocking(store, move |s| s.delete(&key)).await?;
    ctx.config.profiles.remove(&name);
    if ctx.config.default.as_ref() == Some(&name) {
        ctx.config.default = None;
    }
    ctx.save()?;
    if out.json {
        Out::print_json(&serde_json::json!({
            "profile": name.as_str(), "removed": true, "secret_removed": removed, "session_revoked": revoked,
        }));
    } else if profile.kind == Kind::Person {
        Out::note(&format!(
            "Signed out profile {name}: the session is revoked and removed from this machine."
        ));
    } else {
        Out::note(&format!(
            "Removed profile {name} from this machine. An API token stays valid until it expires or is \
             revoked: revoke it in the console, or with `iohr token revoke`."
        ));
    }
    Ok(())
}

fn read_token() -> Result<Redacted<String>, Error> {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return Err(Error::Usage(
            "--with-token reads the token from standard input: `iohr login --with-token < token.txt`".into(),
        ));
    }
    let mut text = Zeroizing::new(String::new());
    stdin
        .lock()
        .take(MAX_TOKEN)
        .read_to_string(&mut text)
        .map_err(|e| Error::Usage(format!("cannot read the token from standard input: {e}")))?;
    let token = text.trim().trim_start_matches("Bearer ").trim();
    if token.is_empty() {
        return Err(Error::Usage(
            "standard input was empty: pipe a token into `iohr login --with-token`".into(),
        ));
    }
    Ok(Redacted::new(token.to_owned()))
}

pub(crate) fn storage_name(s: Storage) -> &'static str {
    match s {
        Storage::File => "an owner-only file",
        _ => "the OS credential store",
    }
}
