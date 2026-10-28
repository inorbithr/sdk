use iohr_auth::{EntryKey, Kind, Profile, ProfileName};
use serde_json::Value;

use crate::Env;
use crate::cli::{Global, ProfileCommand};
use crate::commands::accounts::Me;
use crate::commands::login::storage_name;
use crate::context::{Ctx, blocking, session_for};
use crate::error::Error;
use crate::output::Out;

pub(crate) async fn run(g: &Global, env: &Env, cmd: ProfileCommand, out: Out) -> Result<(), Error> {
    let mut ctx = Ctx::load(g)?;
    match cmd {
        ProfileCommand::Account { name, account } => {
            account_of(g, env, &mut ctx, name, &account, out).await
        }
        ProfileCommand::List => {
            if out.json {
                Out::print_json(&ctx.config);
                return Ok(());
            }
            if ctx.config.profiles.is_empty() {
                Out::note("No profiles yet: `iohr login --with-token < token.txt` adds one.");
                return Ok(());
            }
            let rows: Vec<Vec<String>> = ctx
                .config
                .profiles
                .iter()
                .map(|(name, p)| {
                    let mark = if ctx.config.default.as_ref() == Some(name) {
                        "*"
                    } else {
                        ""
                    };
                    vec![
                        format!("{name}{mark}"),
                        kind(p.kind).into(),
                        p.account.clone(),
                        storage_name(p.storage).into(),
                    ]
                })
                .collect();
            Out::table(&["PROFILE", "KIND", "ACCOUNT", "KEPT IN"], &rows);
            Ok(())
        }
        ProfileCommand::Use { name } => {
            if ctx.config.profile(&name).is_none() {
                return Err(Error::Usage(format!(
                    "there is no profile {name}: `iohr profile list` shows them"
                )));
            }
            ctx.config.default = Some(name.clone());
            ctx.save()?;
            Out::note(&format!("Profile {name} is now the default."));
            Ok(())
        }
        ProfileCommand::Show { name } => show(g, &ctx, name, out).await,
    }
}

/// `iohr profile show [NAME]`: one profile, never its secret.
async fn show(g: &Global, ctx: &Ctx, name: Option<ProfileName>, out: Out) -> Result<(), Error> {
    let name = match name {
        Some(n) => n,
        None => ctx.chosen(g)?,
    };
    let p = ctx
        .config
        .profile(&name)
        .cloned()
        .ok_or_else(|| Error::Usage(format!("there is no profile {name}")))?;
    let key = EntryKey::new(name.clone(), &p.account)?;
    let present = blocking(ctx.store(p.storage)?, move |s| {
        s.get(&key).map(|v| v.is_some())
    })
    .await?;
    let default = ctx.config.default.as_ref() == Some(&name);
    if out.json {
        Out::print_json(&serde_json::json!({
            "profile": name.as_str(), "kind": kind(p.kind), "account": p.account,
            "account_chosen": p.account_pinned,
            "storage": p.storage, "default": default, "credential_present": present,
        }));
    } else {
        let mut pairs = vec![
            ("profile", name.to_string()),
            ("kind", kind(p.kind).into()),
            ("account", p.account),
        ];
        if p.kind == Kind::Person {
            pairs.push((
                "account chosen",
                if p.account_pinned {
                    "yes: a sign-in again keeps it"
                } else {
                    "no: a sign-in again follows your default account"
                }
                .into(),
            ));
        }
        pairs.extend([
            ("kept in", storage_name(p.storage).into()),
            ("default", if default { "yes" } else { "no" }.into()),
            (
                "credential",
                if present {
                    "present"
                } else {
                    "missing: sign in again"
                }
                .into(),
            ),
        ]);
        Out::pairs(&pairs);
    }
    Ok(())
}

/// `iohr profile account NAME ACCOUNT`: point a person's profile at one of their
/// accounts, by id or slug, after checking the API lists it.
async fn account_of(
    g: &Global,
    env: &Env,
    ctx: &mut Ctx,
    name: ProfileName,
    account: &str,
    out: Out,
) -> Result<(), Error> {
    let profile = ctx.config.profile(&name).cloned().ok_or_else(|| {
        Error::Usage(format!(
            "there is no profile {name}: `iohr profile list` shows them"
        ))
    })?;
    if profile.kind != Kind::Person {
        return Err(Error::Usage(format!(
            "profile {name} holds an API token, which is for one account; make another token for the other account"
        )));
    }
    // The person's own accounts, as the API lists them: an id or slug not in the
    // list is refused, so a profile never points at an account it cannot act for.
    let s = session_for(g, env, ctx, Some(&name)).await?;
    let me: Value = s.api.get("/v1/accounts/me", &[]).await?;
    let me = Me::read(&me)
        .ok_or_else(|| Error::Failed("the API listed no account for this profile".into()))?;
    let Some(found) = me.find(account) else {
        return Err(Error::Usage(format!(
            "{account} is not an account {name} belongs to; the ones it does: {}",
            me.listed()
        )));
    };
    // Named by hand: a later sign-in keeps it.
    point_at(ctx, &name, profile, &found.id, true).await?;
    if out.json {
        Out::print_json(
            &serde_json::json!({ "profile": name.as_str(), "account": found.id, "name": found.name, "plan": found.plan }),
        );
    } else {
        Out::note(&format!(
            "Profile {name} now acts for {} ({}, plan {}), and a later `iohr login` keeps it.",
            found.id, found.name, found.plan
        ));
    }
    Ok(())
}

/// Points `profile` at account `id` and saves it. The credential entry is keyed by
/// profile and account, so it moves with the account. `chosen` marks the account as the
/// person's own choice, which a sign-in again keeps.
pub(crate) async fn point_at(
    ctx: &mut Ctx,
    name: &ProfileName,
    profile: Profile,
    id: &str,
    chosen: bool,
) -> Result<(), Error> {
    if id != profile.account {
        let old_key = EntryKey::new(name.clone(), &profile.account)?;
        let new_key = EntryKey::new(name.clone(), id)?;
        let store = ctx.store(profile.storage)?;
        blocking(store, move |st| {
            if let Some(secret) = st.get(&old_key)? {
                st.set(&new_key, &secret)?;
                st.delete(&old_key)?;
            }
            Ok(())
        })
        .await?;
    }
    let mut updated = profile;
    id.clone_into(&mut updated.account);
    updated.account_pinned = chosen;
    ctx.config.profiles.insert(name.clone(), updated);
    ctx.save()
}

fn kind(k: Kind) -> &'static str {
    match k {
        Kind::Person => "person",
        _ => "token",
    }
}
