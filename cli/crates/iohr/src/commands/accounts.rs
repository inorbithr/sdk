use serde_json::Value;

use crate::Env;
use crate::cli::Global;
use crate::context::session;
use crate::error::Error;
use crate::output::Out;

pub(crate) async fn list(g: &Global, env: &Env, out: Out) -> Result<(), Error> {
    let s = session(g, env).await?;
    let me: Value = s.api.get("/v1/accounts/me", &[]).await.map_err(|e| {
        if e.status() == Some(403) {
            Error::with_hint(
                e,
                "An API token reads its accounts only with the account:read scope.",
            )
        } else {
            e.into()
        }
    })?;
    let accounts = accounts_of(&me);
    if out.json {
        Out::print_json(&accounts);
        return Ok(());
    }
    let rows: Vec<Vec<String>> = accounts
        .iter()
        .map(|a| {
            let id = field(a, "id");
            let mark = if id == s.account { "*" } else { "" };
            vec![
                format!("{id}{mark}"),
                field(a, "kind"),
                field(a, "name"),
                field(a, "plan"),
                field(a, "role"),
            ]
        })
        .collect();
    Out::table(&["ACCOUNT", "KIND", "NAME", "PLAN", "ROLE"], &rows);
    Ok(())
}

/// The accounts a `GET /v1/accounts/me` answer lists: the person's own, then their teams.
pub(crate) fn accounts_of(me: &Value) -> Vec<&Value> {
    let mut accounts: Vec<&Value> = me.get("account").into_iter().collect();
    accounts.extend(
        me.get("teams")
            .and_then(Value::as_array)
            .into_iter()
            .flatten(),
    );
    accounts
}

/// A string field of an account object, empty when absent.
pub(crate) fn field(a: &Value, k: &str) -> String {
    a.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// One account a person can act for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Known {
    pub(crate) id: String,
    pub(crate) slug: String,
    pub(crate) name: String,
    pub(crate) plan: String,
}

/// The accounts a person belongs to, and the one they made their default on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Me {
    /// The person's own account first, then their teams.
    pub(crate) accounts: Vec<Known>,
    /// `preferences.default_account_id`; `None` when unset.
    pub(crate) server_default: Option<String>,
}

impl Me {
    /// Reads a `GET /v1/accounts/me` answer; `None` when it names no account of the
    /// person's own (an API key's answer, or a stub).
    pub(crate) fn read(me: &Value) -> Option<Self> {
        me.get("account")?;
        let accounts: Vec<Known> = accounts_of(me)
            .into_iter()
            .map(|a| Known {
                id: field(a, "id"),
                slug: field(a, "slug"),
                name: field(a, "name"),
                plan: field(a, "plan"),
            })
            .filter(|k| !k.id.is_empty())
            .collect();
        if accounts.is_empty() {
            return None;
        }
        let server_default = me
            .pointer("/preferences/default_account_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        Some(Self {
            accounts,
            server_default,
        })
    }

    /// The account with this id or slug.
    pub(crate) fn find(&self, id_or_slug: &str) -> Option<&Known> {
        if id_or_slug.is_empty() {
            return None;
        }
        self.accounts
            .iter()
            .find(|k| k.id == id_or_slug || k.slug == id_or_slug)
    }

    /// The person's own account.
    pub(crate) fn personal(&self) -> &Known {
        &self.accounts[0]
    }

    /// `acc_1 (slug), acc_2`: what a refusal lists.
    pub(crate) fn listed(&self) -> String {
        self.accounts
            .iter()
            .map(|k| {
                if k.slug.is_empty() {
                    k.id.clone()
                } else {
                    format!("{} ({})", k.id, k.slug)
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Why an account was chosen, by the order of the rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Why {
    /// `iohr login --account`.
    Flag,
    /// The profile's own chosen account (`account_pinned`).
    Chosen,
    /// The account the person made their default on the server.
    ServerDefault,
    /// Their own account: nothing else applied.
    Personal,
}

impl Why {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Chosen => "profile",
            Self::ServerDefault => "server_default",
            Self::Personal => "personal",
        }
    }

    /// Whether a sign-in again must keep this choice.
    pub(crate) fn is_chosen(self) -> bool {
        matches!(self, Self::Flag | Self::Chosen)
    }
}

/// The account a profile signs in on, in this order: `--account`, the profile's chosen
/// account, the server's default account, the person's own. `chosen` is the profile's
/// `account` when it is pinned. A flag naming an account the person is not in is an
/// error; a chosen or default account they have left is skipped, with the reason in the
/// second field.
pub(crate) fn choose<'a>(
    me: &'a Me,
    flag: Option<&str>,
    chosen: Option<&str>,
) -> Result<(&'a Known, Why, Option<String>), Error> {
    if let Some(f) = flag {
        let k = me.find(f).ok_or_else(|| {
            Error::Usage(format!(
                "{f} is not an account you belong to; the ones you do: {}",
                me.listed()
            ))
        })?;
        return Ok((k, Why::Flag, None));
    }
    let mut skipped = None;
    if let Some(c) = chosen {
        match me.find(c) {
            Some(k) => return Ok((k, Why::Chosen, None)),
            None => {
                skipped = Some(format!(
                    "the profile's chosen account {c} is not one you belong to now"
                ));
            }
        }
    }
    if let Some(d) = &me.server_default {
        match me.accounts.iter().find(|k| &k.id == d) {
            Some(k) => return Ok((k, Why::ServerDefault, skipped)),
            None => {
                skipped.get_or_insert_with(|| {
                    format!("your default account {d} is not one you belong to")
                });
            }
        }
    }
    Ok((me.personal(), Why::Personal, skipped))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Me, Why, choose};

    fn me(default: &str) -> Me {
        Me::read(&json!({
            "account": {"id": "acc_me", "slug": "", "name": "Ana", "plan": "free"},
            "teams": [
                {"id": "acc_eng", "slug": "engineering", "name": "Engineering", "plan": "team"},
                {"id": "acc_ops", "slug": "ops", "name": "Ops", "plan": "team"},
            ],
            "preferences": {"default_account_id": default},
        }))
        .unwrap()
    }

    fn picked(m: &Me, flag: Option<&str>, chosen: Option<&str>) -> (String, Why) {
        let (k, why, _) = choose(m, flag, chosen).unwrap();
        (k.id.clone(), why)
    }

    #[test]
    fn the_flag_beats_the_chosen_account_the_server_default_and_the_personal_one() {
        let m = me("acc_eng");
        assert_eq!(
            picked(&m, Some("ops"), Some("acc_eng")),
            ("acc_ops".into(), Why::Flag)
        );
        assert_eq!(
            picked(&m, Some("acc_me"), Some("acc_ops")),
            ("acc_me".into(), Why::Flag)
        );
    }

    #[test]
    fn the_chosen_account_beats_the_server_default_and_the_personal_one() {
        let m = me("acc_eng");
        assert_eq!(
            picked(&m, None, Some("acc_ops")),
            ("acc_ops".into(), Why::Chosen)
        );
        assert_eq!(
            picked(&m, None, Some("acc_me")),
            ("acc_me".into(), Why::Chosen)
        );
    }

    #[test]
    fn the_server_default_beats_the_personal_account() {
        assert_eq!(
            picked(&me("acc_eng"), None, None),
            ("acc_eng".into(), Why::ServerDefault)
        );
    }

    #[test]
    fn the_personal_account_is_the_last_resort() {
        assert_eq!(
            picked(&me(""), None, None),
            ("acc_me".into(), Why::Personal)
        );
    }

    #[test]
    fn a_default_or_chosen_account_the_person_has_left_is_skipped_and_said() {
        let gone = me("acc_gone");
        let (k, why, note) = choose(&gone, None, None).unwrap();
        assert_eq!((k.id.as_str(), why), ("acc_me", Why::Personal));
        assert!(note.unwrap().contains("acc_gone"));
        let eng = me("acc_eng");
        let (k, why, note) = choose(&eng, None, Some("acc_gone")).unwrap();
        assert_eq!((k.id.as_str(), why), ("acc_eng", Why::ServerDefault));
        assert!(note.unwrap().contains("acc_gone"));
    }

    #[test]
    fn a_flag_for_an_account_the_person_is_not_in_is_refused_with_the_ones_they_are() {
        let e = choose(&me("acc_eng"), Some("nobody"), None)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("nobody") && e.contains("acc_eng (engineering)") && e.contains("acc_me"),
            "{e}"
        );
        assert!(choose(&me(""), Some(""), None).is_err());
    }

    #[test]
    fn an_answer_without_the_persons_own_account_is_not_read() {
        assert!(Me::read(&json!({"teams": []})).is_none());
        assert!(Me::read(&json!({"account": {"id": ""}})).is_none());
    }
}
