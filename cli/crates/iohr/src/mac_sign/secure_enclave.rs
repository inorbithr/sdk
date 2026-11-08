//! The Secure Enclave key, through a small Swift helper (`helper.swift`).
//!
//! Why a helper: this crate forbids `unsafe`, and a plain command line tool cannot keep an
//! enclave key in the keychain (no entitlement; the system answers -34018). `CryptoKit` can hand
//! out an opaque blob for a key that lives in the enclave, which is what is kept on disk. The
//! helper is compiled once with the system's `swiftc` (Xcode or the command line tools) into
//! the key's folder, named by a hash of its source so a changed source is a new binary, and
//! run for three things: make the key, read its public key, sign a message after Touch ID.
//! The message is built in Rust (`mac_sign::sign_decision`) and handed over on stdin.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest as _, Sha256};

use super::{DeviceSigner, store};

const SOURCE: &str = include_str!("helper.swift");

/// An enclave key and the helper that uses it.
pub(super) struct SecureEnclave {
    helper: PathBuf,
    blob: PathBuf,
}

impl SecureEnclave {
    /// The key in `dir`, with its helper built; the key itself need not exist yet
    /// (`ensure` makes it).
    pub(super) fn open(dir: &Path) -> Result<Self, String> {
        iohr_auth::create_private_dir(dir)
            .map_err(|e| format!("cannot make {}: {}", dir.display(), e.kind()))?;
        Ok(Self {
            helper: build_helper(dir)?,
            blob: store::blob(dir),
        })
    }

    fn run(&self, args: &[&str], extra: &[&str], stdin: Option<&str>) -> Result<String, String> {
        let mut cmd = Command::new(&self.helper);
        cmd.args(args).arg(&self.blob).args(extra);
        cmd.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("cannot start the Secure Enclave helper: {}", e.kind()))?;
        if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
            pipe.write_all(text.as_bytes())
                .map_err(|e| format!("cannot reach the Secure Enclave helper: {}", e.kind()))?;
        }
        let out = child
            .wait_with_output()
            .map_err(|e| format!("the Secure Enclave helper failed: {}", e.kind()))?;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned());
        }
        // The helper's own words: they name the cause, never the message it signed.
        let why = String::from_utf8_lossy(&out.stderr);
        Err(why
            .lines()
            .next()
            .unwrap_or("the Secure Enclave helper failed")
            .to_owned())
    }
}

impl DeviceSigner for SecureEnclave {
    fn ensure(&self) -> Result<String, String> {
        if self.blob.exists() {
            self.public_key()
        } else {
            self.run(&["create"], &[], None)
        }
    }

    fn public_key(&self) -> Result<String, String> {
        self.run(&["public"], &[], None)
    }

    fn sign(&self, message: &str, reason: &str) -> Result<String, String> {
        self.run(&["sign"], &[reason], Some(message))
    }
}

/// `dir/helper-<hash>`, compiled from [`SOURCE`] when it is not there.
fn build_helper(dir: &Path) -> Result<PathBuf, String> {
    let digest = crate::runner::claude::hex(&Sha256::digest(SOURCE.as_bytes()));
    let hash = &digest[..16];
    let out = dir.join(format!("helper-{hash}"));
    if out.is_file() {
        return Ok(out);
    }
    let src = dir.join(format!("helper-{hash}.swift"));
    std::fs::write(&src, SOURCE)
        .map_err(|e| format!("cannot write {}: {}", src.display(), e.kind()))?;
    // Built beside the final name, then moved, so two runs never see half a binary.
    let tmp = dir.join(format!("helper-{hash}.{}.tmp", std::process::id()));
    let built = Command::new("swiftc")
        .arg("-O")
        .arg(&src)
        .arg("-o")
        .arg(&tmp)
        .stdin(Stdio::null())
        .output()
        .map_err(|_| {
            "signing on this Mac needs the Swift compiler: run `xcode-select --install`".to_owned()
        })?;
    if !built.status.success() {
        let _ = std::fs::remove_file(&tmp);
        let why = String::from_utf8_lossy(&built.stderr);
        return Err(format!(
            "the Secure Enclave helper did not build: {}",
            why.lines()
                .find(|l| l.contains("error"))
                .unwrap_or("swiftc failed")
        ));
    }
    std::fs::rename(&tmp, &out).map_err(|e| format!("cannot place the helper: {}", e.kind()))?;
    let _ = std::fs::remove_file(&src);
    Ok(out)
}
