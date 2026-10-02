use iohr_auth::{EntryKey, Kind};

use crate::cli::{Global, ProfileCommand};
use crate::commands::login::storage_name;
use crate::context::{Ctx, blocking};
use crate::error::Error;
use crate::output::Out;

pub(crate) async fn run(g: &Global, cmd: ProfileCommand, out: Out) -> Result<(), Error> {
    let mut ctx = Ctx::load(g)?;
    match cmd {
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
        ProfileCommand::Show { name } => {
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
                    "storage": p.storage, "default": default, "credential_present": present,
                }));
            } else {
                Out::pairs(&[
                    ("profile", name.to_string()),
                    ("kind", kind(p.kind).into()),
                    ("account", p.account),
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
            }
            Ok(())
        }
    }
}

fn kind(k: Kind) -> &'static str {
    match k {
        Kind::Person => "person",
        _ => "token",
    }
}
