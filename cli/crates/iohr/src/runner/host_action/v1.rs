//! `runner/host-action/v1`: the v0 document plus the `provider` that carries it out.
//!
//! `"builtin"` is one of this runner's own actions; without `provider` the rest of the
//! document is exactly a v0 one and is checked as v0 is. An extension provider names an
//! installed extension, its version and its manifest digest, and the action is one that
//! extension declares in its manifest: the document's shape is checked here, and what the
//! extension declares is checked against the installed copy before anything runs
//! ([`super::extension`]).
//!
//! The SDK defines this schema (`tests/data/host-action/v1`); a v0 document still reads.

use serde_json::{Map, Value};

use super::{closed, is_action_id, is_builtin, is_effect, is_runner_id, validate};

/// The schema a v1 run's `schema_id` names.
pub(crate) const HOST_ACTION_V1: &str = "runner/host-action/v1";

/// The most parameters an extension action takes.
pub(crate) const MAX_PARAMS: usize = 8;

/// The largest integer a parameter holds: what every JSON reader keeps exactly.
pub(crate) const MAX_INTEGER: u64 = (1 << 53) - 1;

/// The longest string a parameter holds.
const MAX_STRING: usize = 200;

/// Who carries out an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Provider {
    /// This runner's own catalog.
    Builtin,
    /// An installed extension that declares the action.
    Extension {
        /// The extension's name.
        extension: String,
        /// The version the owner approved.
        version: String,
        /// The manifest digest the owner approved, `sha256:<64 hex>`.
        manifest_digest: String,
    },
}

/// `^sha256:[0-9a-f]{64}$`.
pub(crate) fn is_digest(s: &str) -> bool {
    s.strip_prefix("sha256:").is_some_and(|h| {
        h.len() == 64 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// `^[a-z][a-z0-9_]{0,31}$`: a parameter's name.
pub(crate) fn is_param_name(s: &str) -> bool {
    let mut b = s.bytes();
    b.next().is_some_and(|c| c.is_ascii_lowercase())
        && s.len() <= 32
        && b.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The provider `doc` names: `"builtin"`, or none (a v0 document), is the runner's own.
///
/// # Errors
/// Where the provider breaks the schema, never a value.
pub(crate) fn provider_of(doc: &Value) -> Result<Provider, String> {
    match doc.get("provider") {
        None => Ok(Provider::Builtin),
        Some(Value::String(s)) if s == "builtin" => Ok(Provider::Builtin),
        Some(Value::Object(p)) => {
            closed(p, &["extension", "version", "manifest_digest"], "/provider")?;
            let field = |k: &str| -> Result<String, String> {
                p.get(k)
                    .ok_or_else(|| format!("/provider/{k}: required"))?
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("/provider/{k}: not a string"))
            };
            let (extension, version, manifest_digest) = (
                field("extension")?,
                field("version")?,
                field("manifest_digest")?,
            );
            if crate::ext::manifest::check_name(&extension).is_err() {
                return Err("/provider/extension: not an extension name".to_owned());
            }
            if crate::ext::manifest::check_version(&version).is_err() {
                return Err("/provider/version: not a version".to_owned());
            }
            if !is_digest(&manifest_digest) {
                return Err("/provider/manifest_digest: not a sha256 digest".to_owned());
            }
            Ok(Provider::Extension {
                extension,
                version,
                manifest_digest,
            })
        }
        Some(_) => Err("/provider: \"builtin\" or an extension".to_owned()),
    }
}

/// Whether `doc` is a `runner/host-action/v1` document. A built-in action is the v0
/// document with `provider: "builtin"`, checked as v0; an extension action is checked for
/// shape here and against the installed manifest before it runs. The error names where,
/// never a value.
///
/// # Errors
/// The first rule it breaks.
pub(crate) fn validate_v1(doc: &Value) -> Result<(), String> {
    let Value::Object(m) = doc else {
        return Err("/: not an object".to_owned());
    };
    closed(
        m,
        &["provider", "action", "runner_id", "params", "effect"],
        "/",
    )?;
    if !m.contains_key("provider") {
        return Err("/provider: required".to_owned());
    }
    let Provider::Extension { extension, .. } = provider_of(doc)? else {
        let mut v0 = m.clone();
        v0.remove("provider");
        return validate(&Value::Object(v0));
    };
    for k in ["action", "runner_id", "params", "effect"] {
        if !m.contains_key(k) {
            return Err(format!("/{k}: required"));
        }
    }
    let id = m["action"].as_str().ok_or("/action: not a string")?;
    if !is_action_id(id) || is_builtin(id) {
        return Err("/action: not an extension action id".to_owned());
    }
    if !id
        .strip_prefix(extension.as_str())
        .is_some_and(|rest| rest.starts_with('.'))
    {
        return Err("/action: not an action of the provider".to_owned());
    }
    let runner = m["runner_id"].as_str().ok_or("/runner_id: not a string")?;
    if !is_runner_id(runner) {
        return Err("/runner_id: not a runner id".to_owned());
    }
    let Value::Object(params) = &m["params"] else {
        return Err("/params: not an object".to_owned());
    };
    scalar_params(params)?;
    let effect = m["effect"].as_str().ok_or("/effect: not a string")?;
    if !is_effect(effect) {
        return Err("/effect: 1 to 200 characters on one line".to_owned());
    }
    Ok(())
}

/// An extension action's parameters: at most [`MAX_PARAMS`], each a plain name holding a
/// one-line string or an integer from 0 to [`MAX_INTEGER`]. What each must be is the
/// manifest's to say.
fn scalar_params(params: &Map<String, Value>) -> Result<(), String> {
    if params.len() > MAX_PARAMS {
        return Err(format!("/params: at most {MAX_PARAMS}"));
    }
    for (k, v) in params {
        if !is_param_name(k) {
            return Err("/params: a parameter name the schema does not allow".to_owned());
        }
        let ok = match v {
            Value::String(s) => {
                !s.is_empty()
                    && s.chars().count() <= MAX_STRING
                    && !s.chars().any(char::is_control)
            }
            Value::Number(n) => n.as_u64().is_some_and(|n| n <= MAX_INTEGER),
            _ => false,
        };
        if !ok {
            return Err(format!(
                "/params/{k}: a one-line string or an integer from 0 to {MAX_INTEGER}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
