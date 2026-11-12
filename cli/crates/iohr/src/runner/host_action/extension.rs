//! Where an approved host action goes: this runner's own catalog, or an installed
//! extension that declares it.
//!
//! An extension's action runs only when the installed copy still verifies (program hash,
//! kept bundles, signer), is the version and digest the person approved, declares the
//! action for this system, takes exactly the approved parameters, and renders the approved
//! effect from them. Its program then starts as `<program> host-action <id>`, the
//! parameters as JSON on its standard input: no shell, the runner's scrubbed environment,
//! no token.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::ext::host_actions::HostAction;
use crate::ext::install::{ExtError, Store};
use crate::ext::trust::Policy;

use super::v1::{Provider, provider_of};
use super::{Action, ArgvRunner, clipped, lookup_on, said};

/// The installed extensions a runner carries out host actions for, and the policy each
/// is checked against again before one runs.
#[derive(Debug, Clone)]
pub(crate) struct Extensions {
    store: Store,
    policy: Policy,
}

impl Extensions {
    pub(crate) fn new(store: Store, policy: Policy) -> Self {
        Self { store, policy }
    }

    /// The installed copy of `name`, verified again offline.
    ///
    /// # Errors
    /// It is not installed, or no longer verifies.
    pub(crate) fn installed(&self, name: &str) -> Result<Installed, String> {
        match self.store.verify(name, &self.policy) {
            Ok((entry, record, program)) => Ok(Installed {
                version: entry.version,
                digests: vec![record.index, record.platform_manifest],
                actions: record.manifest.host_actions,
                program,
            }),
            Err(ExtError::Usage(why)) => Err(why),
            Err(_) => Err(format!(
                "the installed copy of {name} no longer verifies: run `iohr ext verify {name}`"
            )),
        }
    }
}

/// What a verified installed extension offers.
#[derive(Debug, Clone)]
pub(crate) struct Installed {
    /// The version the lock pins.
    pub(crate) version: String,
    /// The image index digest and this platform's manifest digest.
    pub(crate) digests: Vec<String>,
    /// The host actions its manifest declares.
    pub(crate) actions: Vec<HostAction>,
    /// The checked program.
    pub(crate) program: PathBuf,
}

/// One extension host action, checked and ready to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Call {
    pub(crate) id: String,
    pub(crate) program: PathBuf,
    /// The approved parameters as a JSON object.
    pub(crate) input: Vec<u8>,
    pub(crate) limit: Duration,
}

/// Who carries out an approved action.
#[derive(Debug)]
pub(crate) enum Routed {
    Builtin(Action),
    Extension(Call),
}

impl Routed {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::Builtin(a) => a.id(),
            Self::Extension(c) => &c.id,
        }
    }
}

/// Why an approved action goes nowhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NotRouted {
    /// The document does not fit what it names.
    Schema(String),
    /// This runner does not carry it out.
    NotHere(String),
}

/// [`route_on`] for this runner's system.
///
/// # Errors
/// What [`route_on`] says.
pub(crate) fn route(doc: &Value, tools: &impl ArgvRunner) -> Result<Routed, NotRouted> {
    route_on(doc, tools, std::env::consts::OS)
}

/// The built-in action `doc` names, or the extension call, for a runner on `os`. The
/// errors name the action, never a parameter's value.
///
/// # Errors
/// The first check that fails.
pub(crate) fn route_on(
    doc: &Value,
    tools: &impl ArgvRunner,
    os: &str,
) -> Result<Routed, NotRouted> {
    let (extension, version, digest) = match provider_of(doc).map_err(NotRouted::Schema)? {
        Provider::Builtin => {
            let action = lookup_on(doc, os).map_err(NotRouted::Schema)?;
            return match tools.refuses(&action) {
                Some(why) => Err(NotRouted::NotHere(why)),
                None => Ok(Routed::Builtin(action)),
            };
        }
        Provider::Extension {
            extension,
            version,
            manifest_digest,
        } => (extension, version, manifest_digest),
    };
    let installed = tools
        .installed_extension(&extension)
        .map_err(NotRouted::NotHere)?;
    if installed.version != version {
        return Err(NotRouted::NotHere(format!(
            "{extension} {version} was approved, but {} is installed",
            installed.version
        )));
    }
    if !installed.digests.contains(&digest) {
        return Err(NotRouted::NotHere(format!(
            "the installed copy of {extension} {version} is not the one approved"
        )));
    }
    let id = doc
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let declared = installed
        .actions
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(|| {
            NotRouted::NotHere(format!("{extension} {version} does not declare {id}"))
        })?;
    if !declared.runs_on(os) {
        return Err(NotRouted::NotHere(format!("{id} does not run on {os}")));
    }
    let empty = Map::new();
    let params = doc
        .get("params")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    declared.accepts(params).map_err(NotRouted::Schema)?;
    if doc.get("effect").and_then(Value::as_str) != Some(declared.render(params).as_str()) {
        return Err(NotRouted::Schema(format!(
            "/effect: not what {id} declares for these parameters"
        )));
    }
    let input = serde_json::to_vec(params)
        .map_err(|_| NotRouted::Schema("/params: not JSON".to_owned()))?;
    Ok(Routed::Extension(Call {
        id: id.to_owned(),
        program: installed.program,
        input,
        limit: Duration::from_secs(declared.timeout_secs),
    }))
}

/// Starts `call`; the answer says whether it was done, with what the program said
/// (redacted, clipped).
///
/// # Errors
/// It did not start, outlived its limit, or exited unsuccessfully.
pub(crate) async fn carry_out(call: &Call, tools: &impl ArgvRunner) -> Result<String, String> {
    let out = tools
        .run_extension(&call.program, &call.id, &call.input, call.limit)
        .await
        .map_err(clipped)?;
    let what = said(&out);
    let head = format!(
        "{}: {}",
        call.id,
        if out.success { "done" } else { "failed" }
    );
    let text = clipped(if what.is_empty() {
        head
    } else {
        format!("{head}\n{what}")
    });
    if out.success { Ok(text) } else { Err(text) }
}

#[cfg(test)]
mod tests;
