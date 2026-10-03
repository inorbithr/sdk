//! `iohr.lock`: what a generated surface was made from, committed beside it. It holds
//! profile names, each cut's plan, scopes and hash, and the operations it held, never
//! an account id's secret or a token. `iohr sdk check` compares it with what the API
//! serves now and says what moved.

use iohr_openapi::{Api, Cut, Operation};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::error::Error;

/// The lock file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Lock {
    /// The command line that wrote it.
    pub(crate) generator: String,
    /// The target language.
    pub(crate) lang: String,
    /// The directory the surface was written to, relative to the lock.
    pub(crate) out: String,
    /// `info.version` of the documents.
    #[serde(default)]
    pub(crate) api_version: String,
    /// The generator options the surface was rendered with, so `iohr sdk check --files`
    /// renders it the same way: `runtime` and `package` when not the language's default.
    #[serde(default, with = "options", skip_serializing_if = "options::is_default")]
    pub(crate) options: iohr_codegen::Options,
    /// One entry per profile.
    pub(crate) profiles: BTreeMap<String, Locked>,
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
            options: iohr_codegen::Options::default(),
            profiles,
        }
    }

    /// The file's text.
    #[must_use]
    pub(crate) fn render(&self) -> String {
        format!("{HEADER}{}", toml::to_string(self).unwrap_or_default())
    }

    /// Reads a lock file.
    pub(crate) fn read(path: &Path) -> Result<Self, Error> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            Error::Usage(format!(
                "cannot read {}: {e}; run `iohr sdk generate` first, or pass --lock",
                path.display()
            ))
        })?;
        Self::parse(&text)
            .map_err(|e| Error::Failed(format!("{} is not an iohr.lock: {e}", path.display())))
    }

    /// A lock's text, also one written before the options table existed, which kept
    /// `runtime` at the top level.
    fn parse(text: &str) -> Result<Self, toml::de::Error> {
        let mut table: toml::Table = toml::from_str(text)?;
        if let Some(runtime) = table.remove("runtime")
            && !table.contains_key("options")
        {
            let mut options = toml::Table::new();
            options.insert("runtime".into(), runtime);
            table.insert("options".into(), toml::Value::Table(options));
        }
        Self::deserialize(table)
    }

    /// What changed between this lock and a fresh `api`, per profile: an empty list
    /// means nothing moved.
    pub(crate) fn drift(&self, api: &Api) -> Vec<Drift> {
        let mut out = Vec::new();
        for (name, locked) in &self.profiles {
            let Some(cut) = api.profiles.get(name) else {
                continue;
            };
            if cut.hash == locked.cut {
                continue;
            }
            let now: Vec<String> = api.operations_of(name).map(Operation::line).collect();
            let added = now
                .iter()
                .filter(|l| !locked.operations.contains(l))
                .cloned()
                .collect();
            let removed = locked
                .operations
                .iter()
                .filter(|l| !now.contains(l))
                .cloned()
                .collect();
            out.push(Drift {
                profile: name.clone(),
                was: locked.cut.clone(),
                now: cut.hash.clone(),
                added,
                removed,
            });
        }
        out
    }
}

/// One profile whose cut moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Drift {
    pub(crate) profile: String,
    pub(crate) was: String,
    pub(crate) now: String,
    pub(crate) added: Vec<String>,
    pub(crate) removed: Vec<String>,
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

    #[test]
    fn a_lock_from_before_the_options_table_still_reads() {
        let text = "generator = \"0.1.0\"\nlang = \"rust\"\nout = \"iohr\"\nruntime = \"my_inorbithr\"\n\n[profiles.ci]\ncut = \"sha256:ab\"\n";
        let lock = super::Lock::parse(text).unwrap();
        assert_eq!(lock.options.runtime.as_deref(), Some("my_inorbithr"));
    }

    use super::{Lock, Locked};

    #[test]
    fn the_lock_round_trips_and_holds_no_secret() {
        let lock = Lock {
            generator: "0.1.0".into(),
            lang: "rust".into(),
            out: "src/iohr".into(),
            api_version: "0.1.0".into(),
            options: iohr_codegen::Options {
                runtime: Some("my_inorbithr".into()),
                ..Default::default()
            },
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
        assert!(
            text.contains("[options]\nruntime = \"my_inorbithr\""),
            "{text}"
        );
        assert_eq!(Lock::parse(&text).unwrap(), lock);
        assert!(!text.contains("secret") && !text.contains("eyJ"));
    }
}

/// The options as a TOML table, `runtime` and `package` only. A lock written before the
/// table existed kept `runtime` at the top level; `Lock::read` moves it in.
mod options {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Default, Serialize, Deserialize)]
    struct Table {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        runtime: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package: Option<String>,
    }

    pub(super) fn is_default(o: &iohr_codegen::Options) -> bool {
        o.runtime.is_none() && o.package.is_none()
    }

    pub(super) fn serialize<S: Serializer>(
        o: &iohr_codegen::Options,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        Table {
            runtime: o.runtime.clone(),
            package: o.package.clone(),
        }
        .serialize(s)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<iohr_codegen::Options, D::Error> {
        let t = Table::deserialize(d)?;
        Ok(iohr_codegen::Options {
            runtime: t.runtime,
            package: t.package,
            in_package: false,
        })
    }
}
