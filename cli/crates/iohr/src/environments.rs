//! The environments the command line can target (RFC 0148): a closed list read from
//! `environments.toml`, production the default, any other name refused.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

const FILE: &str = include_str!("../environments.toml");

/// One environment's settings. An unset address means the built-in production one.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    /// Whether the address below is a stand-in: calls are refused until the caller
    /// names a real one.
    #[serde(default)]
    pub placeholder: bool,
    /// The API origin.
    pub base_url: Option<String>,
    /// The sign-in service.
    pub issuer: Option<String>,
}

/// The whole list.
#[derive(Debug, Deserialize)]
pub struct Environments {
    default: String,
    #[serde(flatten)]
    by_name: BTreeMap<String, Environment>,
}

impl Environments {
    /// The list built into this binary.
    ///
    /// # Panics
    ///
    /// Never for a released build: `environments.toml` is checked by this crate's tests.
    #[must_use]
    #[allow(clippy::expect_used, reason = "an embedded file the tests parse")]
    pub fn builtin() -> &'static Self {
        static LIST: OnceLock<Environments> = OnceLock::new();
        LIST.get_or_init(|| toml::from_str(FILE).expect("environments.toml parses"))
    }

    /// The name used when none is given.
    #[must_use]
    pub fn default_name(&self) -> &str {
        &self.default
    }

    /// Every name, sorted.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }

    /// The environment called `name`, or the list of valid names.
    ///
    /// # Errors
    ///
    /// A name outside the list.
    pub fn get(&self, name: &str) -> Result<&Environment, String> {
        self.by_name.get(name).ok_or_else(|| {
            let all: Vec<_> = self.names().collect();
            format!(
                "unknown environment '{name}'; the environments are {}",
                all.join(", ")
            )
        })
    }
}

/// A clap value parser: the name when it is on the list.
///
/// # Errors
///
/// A name outside the list.
pub fn parse_name(s: &str) -> Result<String, String> {
    Environments::builtin().get(s).map(|_| s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_parses_and_names_production_as_the_default() {
        let list = Environments::builtin();
        assert_eq!(list.default_name(), "production");
        assert!(list.get("production").is_ok());
        assert!(list.get(list.default_name()).is_ok());
    }

    #[test]
    fn production_has_no_address_of_its_own() {
        let p = Environments::builtin().get("production").unwrap();
        assert!(p.base_url.is_none() && p.issuer.is_none() && !p.placeholder);
    }

    #[test]
    fn dev_is_a_labelled_placeholder_that_is_not_a_production_host() {
        let d = Environments::builtin().get("dev").unwrap();
        assert!(d.placeholder);
        let url = d.base_url.as_deref().unwrap();
        assert!(url.ends_with(".invalid"), "{url}");
        assert!(!url.contains("inorbit.hr"), "{url}");
        assert!(!d.issuer.as_deref().unwrap().contains("inorbit.hr"));
    }

    #[test]
    fn an_unknown_name_is_refused_with_the_valid_ones() {
        let e = parse_name("staging").unwrap_err();
        assert!(
            e.contains("staging") && e.contains("dev, production"),
            "{e}"
        );
    }
}
