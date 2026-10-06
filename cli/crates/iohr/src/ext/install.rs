//! Fetching an extension, checking it, and keeping it on this machine.
//!
//! Layout under the data directory (`~/.local/share/iohr/extensions` on Linux):
//!
//! ```text
//! extensions/
//!   iohr-ext.lock                 what is installed: name, version, digest, signer
//!   agent/0.1.0-<digest12>/
//!     record.json                 what was checked at install, and the program's hash
//!     bundles/0.sigstore.json     the Sigstore bundles, kept for `iohr ext verify`
//!     iohr-agent                  the program
//! ```
//!
//! A version is written in a temporary directory and renamed into place, then the lock
//! is updated: a crash never leaves a half-installed extension that the lock names.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::layer;
use super::lock::{Entry, FILE, Lock};
use super::manifest::{CONFIG_MEDIA_TYPE, LAYER_MEDIA_TYPE, MAX_MANIFEST, Manifest};
use super::oci::{Descriptor, Digest, INDEX, ImageManifest, Index, MANIFEST, Platform, Registry};
use super::trust::{self, Policy, Verified};

/// The largest compressed layer downloaded.
const MAX_LAYER: usize = 256 * 1024 * 1024;

/// Why an extension could not be fetched, checked or kept.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExtError {
    /// The command was used wrongly or names something that does not exist here.
    #[error("{0}")]
    Usage(String),
    /// The registry.
    #[error(transparent)]
    Oci(#[from] super::oci::OciError),
    /// The signature, provenance or digest check failed.
    #[error(transparent)]
    Trust(#[from] trust::TrustError),
    /// The artifact is not a valid extension.
    #[error(transparent)]
    Manifest(#[from] super::manifest::ManifestError),
    /// The program could not be taken from the layer.
    #[error(transparent)]
    Layer(#[from] layer::LayerError),
    /// The lock file.
    #[error(transparent)]
    Lock(#[from] super::lock::LockError),
    /// The disk.
    #[error("{0}")]
    Io(String),
}

fn io(what: &str, path: &Path, e: &std::io::Error) -> ExtError {
    ExtError::Io(format!("cannot {what} {}: {e}", path.display()))
}

/// Which version to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Want {
    /// A tag, which must be the version.
    Version(String),
    /// An image index digest, as a lock file pins it.
    Digest(Digest),
}

impl Want {
    fn reference(&self) -> &str {
        match self {
            Self::Version(v) => v,
            Self::Digest(d) => d.as_str(),
        }
    }
}

/// An extension fetched and checked, not yet written anywhere.
#[derive(Debug)]
pub struct Fetched {
    /// Its manifest.
    pub manifest: Manifest,
    /// The image index digest, pinned in the lock.
    pub index: Digest,
    /// This platform's manifest digest.
    pub platform_manifest: Digest,
    /// Who signed it, and its provenance.
    pub verified: Verified,
    program: Vec<u8>,
    bundles: Vec<(Digest, Vec<u8>)>,
}

/// Fetches `name` at `want` for this machine and checks it: digests of every manifest
/// and blob, the signature and provenance against `policy`, the manifest's rules.
/// Nothing is written.
///
/// # Errors
///
/// [`ExtError`] at the first check that fails.
pub async fn fetch(
    registry: &Registry,
    policy: &Policy,
    name: &str,
    want: &Want,
) -> Result<Fetched, ExtError> {
    super::manifest::check_name(name)?;
    let repo = registry.source().repository(name);
    let index_doc = registry.manifest(&repo, want.reference()).await?;
    let index: Index = serde_json::from_slice(&index_doc.bytes).map_err(|e| {
        ExtError::Manifest(super::manifest::ManifestError(format!(
            "{name} {} is not an OCI image index: {e}",
            want.reference()
        )))
    })?;
    let declared = index.media_type.as_deref().unwrap_or(&index_doc.media_type);
    if declared != INDEX {
        return Err(ExtError::Manifest(super::manifest::ManifestError(format!(
            "{name} {} is not an OCI image index of extension artifacts",
            want.reference()
        ))));
    }
    let here = Platform::current();
    let desc = index
        .manifests
        .iter()
        .find(|d| d.platform.as_ref() == Some(&here) && d.media_type == MANIFEST)
        .ok_or_else(|| {
            let there: Vec<String> = index
                .manifests
                .iter()
                .filter_map(|d| d.platform.as_ref().map(ToString::to_string))
                .collect();
            ExtError::Usage(format!(
                "{name} {} has no build for {here} (it has: {})",
                want.reference(),
                if there.is_empty() {
                    "none".into()
                } else {
                    there.join(", ")
                }
            ))
        })?;
    let platform_doc = registry
        .manifest(&repo, &Digest::parse(&desc.digest)?.to_string())
        .await?;
    let image: ImageManifest = serde_json::from_slice(&platform_doc.bytes).map_err(|e| {
        ExtError::Manifest(super::manifest::ManifestError(format!(
            "the {here} manifest is not an OCI image manifest: {e}"
        )))
    })?;
    let (config, layer_desc) = shape(&image)?;

    // Signatures and provenance before any content is used.
    let mut bundles = Vec::new();
    for d in [&platform_doc.digest, &index_doc.digest] {
        for b in registry.bundles(&repo, d).await? {
            bundles.push((d.clone(), b));
        }
    }
    let verified = trust::verify(policy, &bundles)?;

    let manifest = Manifest::parse(&registry.blob(&repo, config, MAX_MANIFEST).await?)?;
    if manifest.name != name {
        return Err(ExtError::Manifest(super::manifest::ManifestError(format!(
            "the artifact says it is {}, not {name}",
            super::manifest::printable(&manifest.name)
        ))));
    }
    if let Want::Version(v) = want
        && &manifest.version != v
    {
        return Err(ExtError::Manifest(super::manifest::ManifestError(format!(
            "the tag {v} holds version {}",
            super::manifest::printable(&manifest.version)
        ))));
    }
    let blob = registry.blob(&repo, layer_desc, MAX_LAYER).await?;
    let program = layer::program(&blob, &manifest.entrypoint)?;
    Ok(Fetched {
        manifest,
        index: index_doc.digest,
        platform_manifest: platform_doc.digest,
        verified,
        program,
        bundles,
    })
}

fn shape(image: &ImageManifest) -> Result<(&Descriptor, &Descriptor), ExtError> {
    let bad = |m: &str| ExtError::Manifest(super::manifest::ManifestError(m.to_owned()));
    if image.media_type.as_deref().is_some_and(|m| m != MANIFEST) {
        return Err(bad("the platform manifest is not an OCI image manifest"));
    }
    if image.config.media_type != CONFIG_MEDIA_TYPE {
        return Err(bad(
            "the artifact is not an iohr extension (its config media type)",
        ));
    }
    match image.layers.as_slice() {
        [l] if l.media_type == LAYER_MEDIA_TYPE => Ok((&image.config, l)),
        _ => Err(bad(
            "an iohr extension has exactly one layer, a gzipped tar holding its program",
        )),
    }
}

/// What `record.json` keeps about an installed version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Record {
    /// The manifest as installed.
    pub manifest: Manifest,
    /// The image index digest.
    pub index: String,
    /// This platform's manifest digest.
    pub platform_manifest: String,
    /// The program's file name in the version directory.
    pub program: String,
    /// The SHA-256 of the program, checked before every run.
    pub program_sha256: String,
    /// The signer's full identity at install.
    pub signer: String,
    /// The provenance's repository and ref, when there was one.
    #[serde(default)]
    pub provenance: Option<String>,
    /// The registry it came from.
    pub registry: String,
    /// When, RFC 3339.
    pub installed_at: String,
    /// The bundles kept: file name and the digest each was attached to.
    #[serde(default)]
    pub bundles: Vec<(String, String)>,
}

/// The extensions on this machine.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// The store at `root`.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The platform's data directory for `iohr`, plus `extensions`.
    ///
    /// # Errors
    ///
    /// [`ExtError::Io`] when the home directory cannot be found.
    pub fn default_root() -> Result<PathBuf, ExtError> {
        use etcetera::app_strategy::{AppStrategy as _, AppStrategyArgs, choose_native_strategy};
        let args = AppStrategyArgs {
            top_level_domain: "hr".into(),
            author: "InOrbit".into(),
            app_name: "iohr".into(),
        };
        choose_native_strategy(args)
            .map(|s| s.data_dir().join("extensions"))
            .map_err(|_| {
                ExtError::Io(
                    "cannot find the home directory to keep extensions in; set IOHR_DATA_DIR"
                        .into(),
                )
            })
    }

    /// This machine's lock.
    #[must_use]
    pub fn lock_path(&self) -> PathBuf {
        self.root.join(FILE)
    }

    /// What is installed.
    ///
    /// # Errors
    ///
    /// [`ExtError::Lock`] when the machine's lock cannot be read.
    pub fn lock(&self) -> Result<Lock, ExtError> {
        Ok(Lock::load(&self.lock_path())?)
    }

    fn version_dir(&self, e: &Entry) -> Result<PathBuf, ExtError> {
        let d = Digest::parse(&e.digest)?;
        Ok(self
            .root
            .join(&e.name)
            .join(format!("{}-{}", e.version, d.short())))
    }

    /// The installed version of `name`: its lock entry, record and directory.
    ///
    /// # Errors
    ///
    /// [`ExtError`] when the lock or the record cannot be read.
    pub fn installed(&self, name: &str) -> Result<Option<(Entry, Record, PathBuf)>, ExtError> {
        let Some(entry) = self.lock()?.entries.remove(name) else {
            return Ok(None);
        };
        let dir = self.version_dir(&entry)?;
        let path = dir.join("record.json");
        let text = std::fs::read(&path).map_err(|e| io("read", &path, &e))?;
        let record: Record = serde_json::from_slice(&text)
            .map_err(|e| ExtError::Io(format!("{} is not readable: {e}", path.display())))?;
        Ok(Some((entry, record, dir)))
    }

    /// Writes a fetched extension and records it in the lock. Returns its entry.
    ///
    /// # Errors
    ///
    /// [`ExtError::Io`] when the disk refuses.
    pub fn install(&self, f: &Fetched, registry: &str) -> Result<Entry, ExtError> {
        let entry = Entry::new(
            &f.manifest.name,
            &f.manifest.version,
            &f.index,
            f.verified.signer.lock_name(),
            f.manifest.privileges.clone(),
        );
        create_private_dir(&self.root)?;
        let tmp = self
            .root
            .join(format!(".tmp-{:016x}", getrandom::u64().unwrap_or(0)));
        let result = self.write_version(f, &tmp, registry, &entry);
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&tmp);
        }
        result?;
        let mut lock = self.lock()?;
        let previous = lock.entries.insert(entry.name.clone(), entry.clone());
        lock.save(&self.lock_path())?;
        if let Some(p) = previous
            && p != entry
            && let Ok(old) = self.version_dir(&p)
        {
            let _ = std::fs::remove_dir_all(old);
        }
        Ok(entry)
    }

    fn write_version(
        &self,
        f: &Fetched,
        tmp: &Path,
        registry: &str,
        entry: &Entry,
    ) -> Result<(), ExtError> {
        create_private_dir(tmp)?;
        let program = f
            .manifest
            .entrypoint
            .rsplit('/')
            .next()
            .unwrap_or("program")
            .to_owned();
        write_file(&tmp.join(&program), &f.program, true)?;
        let bundles_dir = tmp.join("bundles");
        create_private_dir(&bundles_dir)?;
        let mut bundles = Vec::new();
        for (i, (digest, bytes)) in f.bundles.iter().enumerate() {
            let file = format!("{i}.sigstore.json");
            write_file(&bundles_dir.join(&file), bytes, false)?;
            bundles.push((file, digest.to_string()));
        }
        let record = Record {
            manifest: f.manifest.clone(),
            index: f.index.to_string(),
            platform_manifest: f.platform_manifest.to_string(),
            program,
            program_sha256: Digest::of(&f.program).to_string(),
            signer: f.verified.signer.to_string(),
            provenance: f.verified.provenance.clone(),
            registry: registry.to_owned(),
            installed_at: now(),
            bundles,
        };
        let json = serde_json::to_vec_pretty(&record).map_err(|e| ExtError::Io(e.to_string()))?;
        write_file(&tmp.join("record.json"), &json, false)?;
        let dest = self.version_dir(entry)?;
        if let Some(parent) = dest.parent() {
            create_private_dir(parent)?;
        }
        if dest.exists() {
            std::fs::remove_dir_all(&dest).map_err(|e| io("replace", &dest, &e))?;
        }
        std::fs::rename(tmp, &dest).map_err(|e| io("move into place", &dest, &e))
    }

    /// Removes `name` from this machine. Returns whether it was installed.
    ///
    /// # Errors
    ///
    /// [`ExtError`] when the lock cannot be updated.
    pub fn remove(&self, name: &str) -> Result<bool, ExtError> {
        let mut lock = self.lock()?;
        let Some(_) = lock.entries.remove(name) else {
            return Ok(false);
        };
        lock.save(&self.lock_path())?;
        let dir = self.root.join(name);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| io("remove", &dir, &e))?;
        }
        Ok(true)
    }

    /// Checks an installed version again, offline: the program's hash and every kept
    /// bundle against `policy`, and the signer against the lock.
    ///
    /// # Errors
    ///
    /// [`ExtError`] naming what no longer holds.
    pub fn verify(&self, name: &str, policy: &Policy) -> Result<(Entry, Record), ExtError> {
        let (entry, record, dir) = self
            .installed(name)?
            .ok_or_else(|| ExtError::Usage(format!("{name} is not installed")))?;
        check_program(&record, &dir)?;
        check_confirmed(&entry, &record)?;
        let mut bundles = Vec::new();
        for (file, digest) in &record.bundles {
            let path = dir.join("bundles").join(file);
            let bytes = std::fs::read(&path).map_err(|e| io("read", &path, &e))?;
            bundles.push((Digest::parse(digest)?, bytes));
        }
        let pinned = [record.index.as_str(), record.platform_manifest.as_str()];
        if bundles.iter().any(|(d, _)| !pinned.contains(&d.as_str())) {
            return Err(ExtError::Trust(trust::TrustError(
                "a kept bundle names an artifact other than the installed one".into(),
            )));
        }
        let v = trust::verify(policy, &bundles)?;
        if v.signer.lock_name() != entry.signer {
            return Err(ExtError::Trust(trust::TrustError(format!(
                "signed by {}, but the lock pins {}",
                v.signer, entry.signer
            ))));
        }
        Ok((entry, record))
    }
}

/// Refuses a program whose bytes changed since it was installed.
///
/// # Errors
///
/// [`ExtError::Trust`] when the hash differs, [`ExtError::Io`] when it cannot be read.
pub fn check_program(record: &Record, dir: &Path) -> Result<PathBuf, ExtError> {
    let path = dir.join(&record.program);
    let bytes = std::fs::read(&path).map_err(|e| io("read", &path, &e))?;
    if Digest::of(&bytes).as_str() != record.program_sha256 {
        return Err(ExtError::Trust(trust::TrustError(format!(
            "{} changed since it was installed: run `iohr ext install {}` again",
            path.display(),
            record.manifest.name
        ))));
    }
    Ok(path)
}

/// Refuses an installed version that declares a privilege the lock does not record as
/// confirmed (SR-32): `iohr` runs nothing whose privileges the person has not seen.
///
/// # Errors
///
/// [`ExtError::Trust`] naming the privileges.
pub fn check_confirmed(entry: &Entry, record: &Record) -> Result<(), ExtError> {
    let missing = entry.unconfirmed(&record.manifest.privileges);
    if missing.is_empty() {
        Ok(())
    } else {
        Err(ExtError::Trust(trust::TrustError(format!(
            "{} declares privileges that were never confirmed ({}): run `iohr ext install {}` \
             again to see and confirm them",
            record.manifest.name,
            missing.join(", "),
            record.manifest.name
        ))))
    }
}

fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn create_private_dir(dir: &Path) -> Result<(), ExtError> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir).map_err(|e| io("create", dir, &e))
}

fn write_file(path: &Path, bytes: &[u8], program: bool) -> Result<(), ExtError> {
    use std::io::Write as _;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, if program { 0o700 } else { 0o600 });
    #[cfg(not(unix))]
    let _ = program;
    let mut f = o.open(path).map_err(|e| io("create", path, &e))?;
    f.write_all(bytes)
        .and_then(|()| f.sync_all())
        .map_err(|e| io("write", path, &e))
}
