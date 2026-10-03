//! `iohr config set|get|unset`: settings that are not profiles.

use crate::cli::{ConfigCommand, ConfigKey, Global};
use crate::context::Ctx;
use crate::error::Error;
use crate::ext::oci::Source;
use crate::ext::trust::TrustedKey;
use crate::output::Out;

const MAX_KEY_FILE: u64 = 64 * 1024;

pub(crate) fn run(g: &Global, cmd: ConfigCommand, out: Out) -> Result<(), Error> {
    let mut ctx = Ctx::load(g)?;
    match cmd {
        ConfigCommand::Set { key, values } => {
            match key {
                ConfigKey::ExtRegistry => {
                    let [value] = values.as_slice() else {
                        return Err(Error::Usage("ext.registry takes one value".into()));
                    };
                    let source = Source::parse(value)?;
                    ctx.config.ext.registry = Some(source.display_with_scheme());
                }
                ConfigKey::ExtTrustedKeys => {
                    let mut keys = Vec::new();
                    for file in &values {
                        let meta = std::fs::metadata(file)
                            .map_err(|e| Error::Usage(format!("cannot read {file}: {e}")))?;
                        if meta.len() > MAX_KEY_FILE {
                            return Err(Error::Usage(format!(
                                "{file} is too large for a public key"
                            )));
                        }
                        let pem = std::fs::read_to_string(file)
                            .map_err(|e| Error::Usage(format!("cannot read {file}: {e}")))?;
                        if pem.contains("PRIVATE KEY") {
                            return Err(Error::Usage(format!(
                                "{file} holds a private key; give the public key only"
                            )));
                        }
                        let key = TrustedKey::from_pem(&pem)
                            .map_err(|e| Error::Usage(format!("{file}: {e}")))?;
                        Out::note(&format!("Trusting {} from {file}.", key.fingerprint()));
                        keys.push(pem.trim().to_owned());
                    }
                    ctx.config.ext.trusted_keys = keys;
                }
            }
            ctx.save()
        }
        ConfigCommand::Get { key } => {
            let value = value(&ctx, key)?;
            if out.json {
                Out::print_json(&value);
            } else {
                match value {
                    serde_json::Value::Array(v) => {
                        for k in v {
                            Out::raw(format!("{}\n", k.as_str().unwrap_or_default()).as_bytes());
                        }
                    }
                    v => Out::raw(format!("{}\n", v.as_str().unwrap_or_default()).as_bytes()),
                }
            }
            Ok(())
        }
        ConfigCommand::Unset { key } => {
            match key {
                ConfigKey::ExtRegistry => ctx.config.ext.registry = None,
                ConfigKey::ExtTrustedKeys => ctx.config.ext.trusted_keys.clear(),
            }
            ctx.save()
        }
    }
}

/// The effective value: for keys, their fingerprints (the PEM stays in the file).
fn value(ctx: &Ctx, key: ConfigKey) -> Result<serde_json::Value, Error> {
    Ok(match key {
        ConfigKey::ExtRegistry => serde_json::Value::String(
            ctx.config
                .ext
                .registry
                .clone()
                .unwrap_or_else(|| crate::ext::oci::DEFAULT_REGISTRY.to_owned()),
        ),
        ConfigKey::ExtTrustedKeys => serde_json::Value::Array(
            ctx.config
                .ext
                .trusted_keys
                .iter()
                .map(|pem| {
                    TrustedKey::from_pem(pem)
                        .map(|k| serde_json::Value::String(k.fingerprint().to_owned()))
                        .map_err(|e| Error::Failed(format!("ext.trusted_keys: {e}")))
                })
                .collect::<Result<_, _>>()?,
        ),
    })
}
