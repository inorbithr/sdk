use std::io::{IsTerminal as _, Read as _};

use iohr_auth::{Claims, EntryKey, Kind, Profile, ProfileName, Redacted, Storage};
use time::OffsetDateTime;
use zeroize::Zeroizing;

use crate::Env;
use crate::cli::{Global, Login};
use crate::context::{Ctx, blocking};
use crate::error::Error;
use crate::output::Out;

/// The profile a first `login` creates when no `--profile` is given.
const DEFAULT_PROFILE: &str = "default";

/// The largest token read from standard input.
const MAX_TOKEN: u64 = 64 * 1024;

pub(crate) async fn login(g: &Global, env: &Env, args: &Login, out: Out) -> Result<(), Error> {
    if !args.with_token {
        return Err(Error::Usage(
            "signing in with a browser or a device code is not in this version of iohr yet. \
             Create an API token in the console and run `iohr login --with-token < token.txt`."
                .into(),
        ));
    }
    if env.token.is_some() {
        Out::note("note: IOHR_TOKEN is set, and it wins over profiles until it is unset");
    }
    let token = read_token()?;
    let claims = Claims::read(token.expose(), OffsetDateTime::now_utc())
        .map_err(|e| Error::Usage(e.to_string()))?;
    let account = claims
        .org
        .clone()
        .ok_or_else(|| Error::Usage("this token names no account (no org claim)".into()))?;

    let mut ctx = Ctx::load(g)?;
    let name = match &g.profile {
        Some(n) => n.clone(),
        None => match ctx.config.default.clone() {
            Some(n) => n,
            None => DEFAULT_PROFILE
                .parse::<ProfileName>()
                .map_err(|e| Error::Failed(e.to_string()))?,
        },
    };
    let storage = if args.insecure_storage {
        Storage::File
    } else {
        Storage::Keyring
    };
    let key = EntryKey::new(name.clone(), &account)?;

    // A profile that pointed at another account or store loses its old secret.
    if let Some(old) = ctx.config.profile(&name).cloned()
        && (old.account != account || old.storage != storage)
    {
        let old_key = EntryKey::new(name.clone(), &old.account)?;
        blocking(ctx.store(old.storage)?, move |s| s.delete(&old_key)).await?;
    }
    blocking(ctx.store(storage)?, move |s| s.set(&key, &token)).await?;
    ctx.config.profiles.insert(
        name.clone(),
        Profile::new(Kind::Token, account.clone(), storage),
    );
    if ctx.config.default.is_none() {
        ctx.config.default = Some(name.clone());
    }
    ctx.save()?;

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

pub(crate) async fn logout(g: &Global, out: Out) -> Result<(), Error> {
    let mut ctx = Ctx::load(g)?;
    let name = ctx.chosen(g)?;
    let profile = ctx
        .config
        .profiles
        .remove(&name)
        .ok_or_else(|| Error::Usage(format!("there is no profile {name}")))?;
    let key = EntryKey::new(name.clone(), &profile.account)?;
    let removed = blocking(ctx.store(profile.storage)?, move |s| s.delete(&key)).await?;
    if ctx.config.default.as_ref() == Some(&name) {
        ctx.config.default = None;
    }
    ctx.save()?;
    if out.json {
        Out::print_json(
            &serde_json::json!({ "profile": name.as_str(), "removed": true, "secret_removed": removed }),
        );
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
