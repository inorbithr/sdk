//! `iohr.lock`: what a generated surface was made from, committed beside it. It holds
//! profile names, each cut's plan, scopes and hash, and the operations it held, never
//! an account id's secret or a token. `iohr sdk check` compares it with what the API
//! serves now and says what moved.

use iohr_openapi::{Api, Cut, Operation};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The lock file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Lock {
    /// The command line that wrote it.
    pub generator: String,
    /// The target language.
    pub lang: String,
    /// The directory the surface was written to, relative to the lock.
    pub out: String,
    /// `info.version` of the documents.
    #[serde(default)]
    pub api_version: String,
    /// One entry per profile.
    pub profiles: BTreeMap<String, Locked>,
}

/// One profile's cut, as it was when the surface was generated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Locked {
    /// `info.x-iohr-cut.hash`, or the hash computed locally when the document had none.
    pub cut: String,
    /// The plan the document was cut by.
    #[serde(default)]
    pub plan: String,
    /// The account, when the gateway said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The scopes it was narrowed to.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Every operation the cut held, as `METHOD /path`.
    #[serde(default)]
    pub operations: Vec<String>,
}

const HEADER: &str = "# iohr.lock: what this surface was generated from. Commit it beside the surface;\n# `iohr sdk check` compares it with what the API serves now and says what moved.\n\n";

impl Lock {
    /// The lock for `api`, rendered by the command line of `generator` into `out`.
    #[must_use]
    pub(crate) fn from_api(api: &Api, lang: &str, out: &str, generator: &str) -> Self {
        let profiles = api
            .profiles
            .iter()
            .map(|(name, cut)| {
                (
                    name.clone(),
                    Locked::from_cut(cut, api.operations_of(name).map(Operation::line).collect()),
                )
            })
            .collect();
        Self {
            generator: generator.to_owned(),
            lang: lang.to_owned(),
            out: out.to_owned(),
            api_version: api.api_version.clone(),
            profiles,
        }
    }

    /// The file's text.
    #[must_use]
    pub(crate) fn render(&self) -> String {
        format!("{HEADER}{}", toml::to_string(self).unwrap_or_default())
    }
}

impl Locked {
    fn from_cut(cut: &Cut, operations: Vec<String>) -> Self {
        Self {
            cut: cut.hash.clone(),
            plan: cut.plan.clone(),
            account: cut.account.clone(),
            scopes: cut.scopes.clone(),
            operations,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Lock, Locked};

    #[test]
    fn the_lock_round_trips_and_holds_no_secret() {
        let lock = Lock {
            generator: "0.1.0".into(),
            lang: "rust".into(),
            out: "src/iohr".into(),
            api_version: "0.1.0".into(),
            profiles: BTreeMap::from([(
                "acme-ci".to_owned(),
                Locked {
                    cut: "sha256:ab".into(),
                    plan: "free".into(),
                    account: Some("acc_1".into()),
                    scopes: vec!["radar:read".into()],
                    operations: vec!["GET /v1/radar/digests".into()],
                },
            )]),
        };
        let text = lock.render();
        assert!(text.starts_with("# iohr.lock"));
        assert!(text.contains("[profiles.acme-ci]"));
        assert_eq!(toml::from_str::<Lock>(&text).unwrap(), lock);
        assert!(!text.contains("secret") && !text.contains("eyJ"));
    }
}
