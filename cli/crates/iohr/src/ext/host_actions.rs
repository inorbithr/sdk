//! `host_actions`: the actions an extension offers a runner's host, each one a person
//! approves before it runs (config v2).
//!
//! A declaration names the action (`<extension>.<action>`), the parameters it takes, each
//! of a closed set of types, the systems it runs on, the effect a person reads with the
//! parameters filled in, and how long it may take. The runner checks an approved
//! document against the installed declaration and then starts the extension's program as
//! `<program> host-action <id>`, the checked parameters as JSON on its standard input:
//! no shell, the runner's own scrubbed environment.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::manifest::ManifestError;
use crate::runner::host_action::v1::{MAX_INTEGER, MAX_PARAMS, is_param_name};
use crate::runner::host_action::{
    is_action_id, is_builtin, is_dns_label, is_effect, is_rev, is_word,
};
use crate::runner::workspace::is_repository_part;

/// The most host actions one extension declares.
pub const MAX_HOST_ACTIONS: usize = 32;
/// The longest a host action may run, in seconds.
pub const MAX_TIMEOUT_SECS: u64 = 3600;
/// The most values an `enum` parameter lists.
pub const MAX_ENUM_VALUES: usize = 32;
/// The systems a host action may name, as the runner reports its own.
pub const SYSTEMS: &[&str] = &["linux", "macos", "windows"];
/// The longest `word` parameter.
const MAX_WORD: usize = 64;

/// One host action an extension declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct HostAction {
    /// `<extension>.<action>`: lower-case words joined by `-`, at most 64 characters.
    pub id: String,
    /// The parameters it takes, each required; at most 8.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, ParamType>,
    /// The systems it runs on: `linux`, `macos` or `windows`.
    pub os: Vec<String>,
    /// What it does, in one line a person reads before approving it; `{name}` stands for
    /// a parameter's value.
    pub effect: String,
    /// How long it may run before the runner stops it, 1 to 3600 seconds.
    pub timeout_secs: u64,
}

/// What a parameter may hold. Each type is closed: nothing a shell or a path reads
/// specially fits in one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", try_from = "RawParam")]
#[non_exhaustive]
pub enum ParamType {
    /// Lower-case letters and digits, words joined by `-`, at most 64 characters.
    Word,
    /// A DNS label: 1 to 63 lower-case letters, digits and `-`, a letter or digit at
    /// each end.
    DnsLabel,
    /// A full commit: 40 lower-case hexadecimal digits.
    Rev,
    /// A repository, `owner/name`: each part 1 to 100 letters, digits, `-`, `_` and `.`.
    Repo,
    /// An integer from `min` to `max`, at most 2^53 - 1.
    Integer {
        /// The smallest value.
        min: u64,
        /// The largest value.
        max: u64,
    },
    /// One of the listed values, each 1 to 64 lower-case letters, digits, `:`, `.`, `_`
    /// and `-`, starting with a letter or digit.
    Enum {
        /// The values it may hold.
        values: Vec<String>,
    },
}

/// A parameter type as written. Serde lets a tagged unit variant carry unknown fields, so
/// the declaration reads through this closed form and each type takes only its own.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawParam {
    #[serde(rename = "type")]
    kind: String,
    min: Option<u64>,
    max: Option<u64>,
    values: Option<Vec<String>>,
}

impl TryFrom<RawParam> for ParamType {
    type Error = String;

    fn try_from(r: RawParam) -> Result<Self, String> {
        let bare = |t: Self| {
            if r.min.is_none() && r.max.is_none() && r.values.is_none() {
                Ok(t)
            } else {
                Err(format!("a `{}` parameter takes no other field", r.kind))
            }
        };
        match (r.kind.as_str(), r.min, r.max, &r.values) {
            ("word", ..) => bare(Self::Word),
            ("dns-label", ..) => bare(Self::DnsLabel),
            ("rev", ..) => bare(Self::Rev),
            ("repo", ..) => bare(Self::Repo),
            ("integer", Some(min), Some(max), None) => Ok(Self::Integer { min, max }),
            ("integer", ..) => Err("an `integer` parameter takes `min` and `max`".to_owned()),
            ("enum", None, None, Some(values)) => Ok(Self::Enum {
                values: values.clone(),
            }),
            ("enum", ..) => Err("an `enum` parameter takes `values`".to_owned()),
            _ => Err("a parameter type is word, dns-label, rev, repo, integer or enum".to_owned()),
        }
    }
}

impl ParamType {
    /// The words an error and the installer use for it.
    #[must_use]
    pub fn words(&self) -> String {
        match self {
            Self::Word => "a word".to_owned(),
            Self::DnsLabel => "a DNS label".to_owned(),
            Self::Rev => "a full commit".to_owned(),
            Self::Repo => "a repository owner/name".to_owned(),
            Self::Integer { min, max } => format!("an integer from {min} to {max}"),
            Self::Enum { values } => format!("one of {}", values.join(", ")),
        }
    }

    fn holds(&self, v: &Value) -> bool {
        match (self, v) {
            (Self::Word, Value::String(s)) => s.len() <= MAX_WORD && is_word(s),
            (Self::DnsLabel, Value::String(s)) => is_dns_label(s),
            (Self::Rev, Value::String(s)) => is_rev(s),
            (Self::Repo, Value::String(s)) => s
                .split_once('/')
                .is_some_and(|(o, n)| is_repository_part(o) && is_repository_part(n)),
            (Self::Integer { min, max }, Value::Number(n)) => {
                n.as_u64().is_some_and(|n| (*min..=*max).contains(&n))
            }
            (Self::Enum { values }, Value::String(s)) => values.iter().any(|x| x == s),
            _ => false,
        }
    }

    fn check(&self, at: &str) -> Result<(), ManifestError> {
        match self {
            Self::Integer { min, max } if min > max || *max > MAX_INTEGER => Err(ManifestError(
                format!("{at}: an integer's min is at most its max, and max at most {MAX_INTEGER}"),
            )),
            Self::Enum { values } => {
                let ok = (1..=MAX_ENUM_VALUES).contains(&values.len())
                    && values.iter().all(|v| is_enum_value(v))
                    && values
                        .iter()
                        .enumerate()
                        .all(|(i, v)| !values[..i].contains(v));
                if ok {
                    Ok(())
                } else {
                    Err(ManifestError(format!(
                        "{at}: an enum lists 1 to {MAX_ENUM_VALUES} different values, each 1 to 64 \
                         lower-case letters, digits, ':', '.', '_' and '-'"
                    )))
                }
            }
            _ => Ok(()),
        }
    }
}

/// `^[a-z0-9][a-z0-9:._-]{0,63}$`.
fn is_enum_value(s: &str) -> bool {
    let mut b = s.bytes();
    b.next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.len() <= 64
        && b.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b":._-".contains(&c))
}

/// One piece of an effect template.
enum Piece<'a> {
    Text(&'a str),
    Param(&'a str),
}

/// The template's pieces, or `None` when a brace does not open or close a parameter name.
fn pieces(template: &str) -> Option<Vec<Piece<'_>>> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        if rest.as_bytes()[i] == b'}' {
            return None;
        }
        out.push(Piece::Text(&rest[..i]));
        let after = &rest[i + 1..];
        let j = after.find('}')?;
        let name = &after[..j];
        if !is_param_name(name) {
            return None;
        }
        out.push(Piece::Param(name));
        rest = &after[j + 1..];
    }
    out.push(Piece::Text(rest));
    Some(out)
}

impl HostAction {
    /// Whether this declaration keeps the rules, for an extension called `extension`.
    ///
    /// # Errors
    /// [`ManifestError`] naming the action and the rule.
    pub fn check(&self, extension: &str) -> Result<(), ManifestError> {
        let bad = |m: String| Err(ManifestError(m));
        if !is_action_id(&self.id)
            || !self
                .id
                .strip_prefix(extension)
                .is_some_and(|rest| rest.starts_with('.'))
        {
            return bad(format!(
                "a host action's id is `{extension}.<action>`, lower-case words joined by '-', \
                 at most 64 characters"
            ));
        }
        let at = format!("host action {}", self.id);
        if is_builtin(&self.id) {
            return bad(format!("{at}: the id is one of the runner's own actions"));
        }
        if self.params.len() > MAX_PARAMS {
            return bad(format!("{at}: at most {MAX_PARAMS} parameters"));
        }
        for (name, t) in &self.params {
            if !is_param_name(name) {
                return bad(format!(
                    "{at}: a parameter name is 1 to 32 lower-case letters, digits and '_', \
                     starting with a letter"
                ));
            }
            t.check(&format!("{at}, parameter {name}"))?;
        }
        let known = self
            .os
            .iter()
            .enumerate()
            .all(|(i, o)| SYSTEMS.contains(&o.as_str()) && !self.os[..i].contains(o));
        if self.os.is_empty() || !known {
            return bad(format!(
                "{at}: os lists one or more of {}",
                SYSTEMS.join(", ")
            ));
        }
        if !(1..=MAX_TIMEOUT_SECS).contains(&self.timeout_secs) {
            return bad(format!("{at}: timeout_secs is 1 to {MAX_TIMEOUT_SECS}"));
        }
        let declared = pieces(&self.effect).is_some_and(|p| {
            p.iter().all(|piece| match piece {
                Piece::Param(n) => self.params.contains_key(*n),
                Piece::Text(_) => true,
            })
        });
        if !is_effect(&self.effect) || !declared {
            return bad(format!(
                "{at}: the effect is one line of at most 200 characters, and each {{name}} in \
                 it is a declared parameter"
            ));
        }
        Ok(())
    }

    /// Whether `params` are exactly this action's parameters, each of its type. The error
    /// names the parameter, never its value.
    ///
    /// # Errors
    /// The first parameter that is missing, unknown or of the wrong type.
    pub fn accepts(&self, params: &Map<String, Value>) -> Result<(), String> {
        for k in params.keys() {
            if !self.params.contains_key(k) {
                return Err(if is_param_name(k) {
                    format!("/params/{k}: not a parameter of {}", self.id)
                } else {
                    format!("/params: a parameter {} does not take", self.id)
                });
            }
        }
        for (k, t) in &self.params {
            let v = params
                .get(k)
                .ok_or_else(|| format!("/params/{k}: required"))?;
            if !t.holds(v) {
                return Err(format!("/params/{k}: not {}", t.words()));
            }
        }
        Ok(())
    }

    /// The effect with each `{name}` replaced by its value in `params`.
    #[must_use]
    pub fn render(&self, params: &Map<String, Value>) -> String {
        let Some(pieces) = pieces(&self.effect) else {
            return self.effect.clone();
        };
        pieces
            .into_iter()
            .map(|p| match p {
                Piece::Text(t) => t.to_owned(),
                Piece::Param(n) => match params.get(n) {
                    Some(Value::String(s)) => s.clone(),
                    Some(v) => v.to_string(),
                    None => format!("{{{n}}}"),
                },
            })
            .collect()
    }

    /// Whether it runs on `os`, as `std::env::consts::OS` names it.
    #[must_use]
    pub fn runs_on(&self, os: &str) -> bool {
        self.os.iter().any(|o| o == os)
    }
}

/// Whether an extension's host actions keep the rules: at most [`MAX_HOST_ACTIONS`],
/// each id once, each declaration valid.
///
/// # Errors
/// [`ManifestError`] naming the rule.
pub fn check_all(actions: &[HostAction], extension: &str) -> Result<(), ManifestError> {
    if actions.len() > MAX_HOST_ACTIONS {
        return Err(ManifestError(format!(
            "an extension declares at most {MAX_HOST_ACTIONS} host actions"
        )));
    }
    for (i, a) in actions.iter().enumerate() {
        a.check(extension)?;
        if actions[..i].iter().any(|b| b.id == a.id) {
            return Err(ManifestError(format!(
                "host action {} is declared twice",
                a.id
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn restart() -> HostAction {
        serde_json::from_value(json!({
            "id": "example.restart-worker",
            "params": {
                "worker": {"type": "dns-label"},
                "grace_secs": {"type": "integer", "min": 0, "max": 600},
            },
            "os": ["linux", "macos"],
            "effect": "Restarts worker {worker} after {grace_secs} seconds.",
            "timeout_secs": 120,
        }))
        .unwrap()
    }

    fn params(v: Value) -> Map<String, Value> {
        let Value::Object(m) = v else { panic!() };
        m
    }

    #[test]
    fn a_declaration_checks_renders_and_accepts_only_its_typed_parameters() {
        let a = restart();
        assert_eq!(a.check("example"), Ok(()));
        let p = params(json!({"worker": "build-1", "grace_secs": 30}));
        assert_eq!(a.accepts(&p), Ok(()));
        assert_eq!(a.render(&p), "Restarts worker build-1 after 30 seconds.");
        assert!(a.runs_on("linux") && !a.runs_on("windows"));
        for (bad, at) in [
            (json!({"worker": "build-1"}), "/params/grace_secs: required"),
            (
                json!({"worker": "build-1", "grace_secs": 601}),
                "/params/grace_secs: not an integer from 0 to 600",
            ),
            (
                json!({"worker": "Build 1; rm", "grace_secs": 1}),
                "/params/worker: not a DNS label",
            ),
            (
                json!({"worker": "b", "grace_secs": 1, "extra": "x"}),
                "/params/extra: not a parameter of example.restart-worker",
            ),
        ] {
            let why = a.accepts(&params(bad)).unwrap_err();
            assert_eq!(why, at);
            assert!(!why.contains("rm"));
        }
    }

    #[test]
    fn every_param_type_holds_only_its_values() {
        let t = |v: Value| -> ParamType { serde_json::from_value(v).unwrap() };
        let cases = [
            (t(json!({"type": "word"})), json!("ok-1"), json!("a b")),
            (
                t(json!({"type": "rev"})),
                json!("a".repeat(40)),
                json!("HEAD"),
            ),
            (
                t(json!({"type": "repo"})),
                json!("acme/app.git"),
                json!("../x"),
            ),
            (
                t(json!({"type": "enum", "values": ["dev", "prod:eu"]})),
                json!("prod:eu"),
                json!("staging"),
            ),
        ];
        for (ty, good, bad) in cases {
            assert!(ty.holds(&good) && !ty.holds(&bad), "{ty:?}");
        }
        assert!(serde_json::from_value::<ParamType>(json!({"type": "path"})).is_err());
        for bad in [
            json!({"type": "word", "x": 1}),
            json!({"type": "word", "min": 1}),
            json!({"type": "integer", "min": 1}),
            json!({"type": "enum", "values": ["a"], "max": 2}),
        ] {
            assert!(
                serde_json::from_value::<ParamType>(bad.clone()).is_err(),
                "{bad}"
            );
        }
        let int = json!({"type": "integer", "min": 1, "max": 9});
        assert_eq!(serde_json::to_value(t(int.clone())).unwrap(), int);
    }

    #[test]
    fn a_declaration_that_breaks_a_rule_is_refused_by_name() {
        let refused = |edit: &dyn Fn(&mut HostAction), ext: &str| {
            let mut a = restart();
            edit(&mut a);
            a.check(ext).unwrap_err().0
        };
        assert!(refused(&|_| {}, "other").contains("`other.<action>`"));
        assert!(refused(&|a| a.id = "example".into(), "example").contains("`example.<action>`"));
        assert!(refused(&|a| a.os = vec![], "example").contains("os lists"));
        assert!(
            refused(&|a| a.os = vec!["linux".into(), "linux".into()], "example")
                .contains("os lists")
        );
        assert!(refused(&|a| a.timeout_secs = 0, "example").contains("timeout_secs"));
        assert!(
            refused(&|a| a.effect = "Restarts {nobody}.".into(), "example")
                .contains("declared parameter")
        );
        assert!(refused(&|a| a.effect = "Two\nlines".into(), "example").contains("one line"));
        assert!(refused(&|a| a.effect = "Stray } brace".into(), "example").contains("effect"));
        assert!(
            refused(
                &|a| {
                    a.params.insert(
                        "n".into(),
                        ParamType::Integer {
                            min: 0,
                            max: MAX_INTEGER + 1,
                        },
                    );
                },
                "example"
            )
            .contains("parameter n")
        );
        let builtin = HostAction {
            id: "docker.prune-images".into(),
            ..restart()
        };
        assert!(
            builtin
                .check("docker")
                .unwrap_err()
                .0
                .contains("runner's own")
        );
        let twice = [restart(), restart()];
        assert!(
            check_all(&twice, "example")
                .unwrap_err()
                .0
                .contains("twice")
        );
    }
}
