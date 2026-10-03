//! An extension's manifest: the OCI config blob of each platform's artifact.

use serde::{Deserialize, Serialize};

/// The config media type of an extension artifact.
pub const CONFIG_MEDIA_TYPE: &str = "application/vnd.inorbit.iohr.extension.config.v1+json";
/// The media type of the one layer, a gzipped tar that holds the program.
pub const LAYER_MEDIA_TYPE: &str = "application/vnd.inorbit.iohr.extension.layer.v1.tar+gzip";

/// The largest manifest read.
pub const MAX_MANIFEST: usize = 64 * 1024;
const MAX_SCOPES: usize = 64;
const MAX_DESCRIPTION: usize = 500;

/// Top-level commands of `iohr` itself, which no extension may take.
pub const RESERVED: &[&str] = &[
    "accounts",
    "api",
    "completion",
    "config",
    "domains",
    "ext",
    "help",
    "iohr",
    "login",
    "logout",
    "openapi",
    "profile",
    "sdk",
    "token",
    "whoami",
];

/// What an extension says about itself: who it is, what to run, which scopes it may ask
/// for. Read from the registry before anything is installed, and shown to the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Manifest {
    /// The command it adds: `iohr <name> ...`.
    pub name: String,
    /// Its version, `MAJOR.MINOR.PATCH` with an optional pre-release.
    pub version: String,
    /// The program's path inside the layer.
    pub entrypoint: String,
    /// The API scopes it may ask `iohr` for; a token request for any other is refused.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// One line about what it does.
    #[serde(default)]
    pub description: String,
}

/// Why a manifest or a name was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ManifestError(pub String);

impl Manifest {
    /// Reads and checks a manifest.
    ///
    /// # Errors
    ///
    /// [`ManifestError`] when the bytes are too large, not the JSON shape, or any field
    /// breaks its rule.
    pub fn parse(bytes: &[u8]) -> Result<Self, ManifestError> {
        if bytes.len() > MAX_MANIFEST {
            return Err(ManifestError(format!(
                "the extension manifest is larger than {} KiB",
                MAX_MANIFEST / 1024
            )));
        }
        let m: Self = serde_json::from_slice(bytes)
            .map_err(|e| ManifestError(format!("the extension manifest is not valid: {e}")))?;
        m.check()?;
        Ok(m)
    }

    fn check(&self) -> Result<(), ManifestError> {
        check_name(&self.name)?;
        check_version(&self.version)?;
        check_entrypoint(&self.entrypoint)?;
        if self.scopes.len() > MAX_SCOPES {
            return Err(ManifestError(format!(
                "an extension declares at most {MAX_SCOPES} scopes"
            )));
        }
        for s in &self.scopes {
            check_scope(s)?;
        }
        if self.description.chars().count() > MAX_DESCRIPTION
            || self.description.chars().any(char::is_control)
        {
            return Err(ManifestError(format!(
                "an extension's description is one line of at most {MAX_DESCRIPTION} characters"
            )));
        }
        Ok(())
    }
}

/// Checks an extension name: 1 to 32 lower-case letters, digits and `-`, starting with
/// a letter, and not one of `iohr`'s own commands.
///
/// # Errors
///
/// [`ManifestError`] naming the rule.
pub fn check_name(name: &str) -> Result<(), ManifestError> {
    let ok = (1..=32).contains(&name.len())
        && name.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.ends_with('-');
    if !ok {
        return Err(ManifestError(
            "an extension name has 1 to 32 lower-case letters, digits and '-', and starts with a letter"
                .into(),
        ));
    }
    if RESERVED.contains(&name) {
        return Err(ManifestError(format!(
            "`{name}` is a command of iohr itself and cannot be an extension"
        )));
    }
    Ok(())
}

/// Checks a version: semantic versioning without build metadata, at most 64 characters.
///
/// # Errors
///
/// [`ManifestError`] when it is not one.
pub fn check_version(version: &str) -> Result<semver::Version, ManifestError> {
    let v = (version.len() <= 64)
        .then(|| semver::Version::parse(version).ok())
        .flatten()
        .filter(|v| v.build.is_empty())
        .ok_or_else(|| {
            ManifestError(format!(
                "`{}` is not a version such as 1.2.3",
                printable(version)
            ))
        })?;
    Ok(v)
}

fn check_entrypoint(e: &str) -> Result<(), ManifestError> {
    let parts: Vec<&str> = e.split('/').collect();
    let ok = (1..=256).contains(&e.len())
        && parts.iter().all(|p| {
            !p.is_empty()
                && *p != "."
                && *p != ".."
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        });
    if ok {
        Ok(())
    } else {
        Err(ManifestError(
            "an extension's entrypoint is a relative path inside its layer, without `..`".into(),
        ))
    }
}

/// Checks a scope: `resource:action`, lower-case.
///
/// # Errors
///
/// [`ManifestError`] when it is not one.
pub fn check_scope(s: &str) -> Result<(), ManifestError> {
    let part = |p: &str| {
        !p.is_empty()
            && p.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
            && p.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.')
            })
    };
    match s.split_once(':') {
        Some((r, a)) if s.len() <= 64 && part(r) && part(a) => Ok(()),
        _ => Err(ManifestError(format!(
            "`{}` is not a scope such as agents:write",
            printable(s)
        ))),
    }
}

/// A value from the registry, safe to put in a message: no control characters, at most
/// 64 characters.
pub(crate) fn printable(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(64).collect()
}

#[cfg(test)]
mod tests {
    use super::{Manifest, check_name};

    fn manifest(v: &serde_json::Value) -> Result<Manifest, String> {
        Manifest::parse(v.to_string().as_bytes()).map_err(|e| e.0)
    }

    #[test]
    fn reads_the_agent_manifest() {
        let m = manifest(&serde_json::json!({
            "name": "agent", "version": "0.1.0", "entrypoint": "iohr-agent",
            "scopes": ["agents:write", "domains:read"], "description": "The InOrbit agent",
            "later": "an unknown field is ignored"
        }))
        .unwrap();
        assert_eq!(m.scopes, ["agents:write", "domains:read"]);
    }

    #[test]
    fn refuses_what_breaks_a_rule() {
        let base =
            serde_json::json!({"name": "agent", "version": "1.0.0", "entrypoint": "bin/agent"});
        for (field, bad) in [
            ("name", serde_json::json!("login")),
            ("name", serde_json::json!("Agent")),
            ("name", serde_json::json!("a/b")),
            ("version", serde_json::json!("1.0")),
            ("version", serde_json::json!("1.0.0+build")),
            ("entrypoint", serde_json::json!("../agent")),
            ("entrypoint", serde_json::json!("/usr/bin/agent")),
            ("entrypoint", serde_json::json!("bin//agent")),
            ("scopes", serde_json::json!(["Agents:write"])),
            ("scopes", serde_json::json!(["agents"])),
            ("description", serde_json::json!("two\nlines")),
            ("description", serde_json::json!("\u{1b}[31mred")),
        ] {
            let mut v = base.clone();
            v[field] = bad.clone();
            assert!(manifest(&v).is_err(), "{field} = {bad}");
        }
        assert!(manifest(&base).is_ok());
        assert!(Manifest::parse(&vec![b' '; 70 * 1024]).is_err());
    }

    #[test]
    fn names() {
        for ok in ["agent", "a", "load-test2"] {
            assert!(check_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "ext", "sdk", "-a", "a-", "1a", "a_b", &"a".repeat(33)] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }
}
