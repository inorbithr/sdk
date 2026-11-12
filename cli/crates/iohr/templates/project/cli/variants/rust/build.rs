//! Stamps the commit into the binary: the checkout's HEAD, else the CI's GITHUB_SHA, else
//! "unknown". `@@NAME@@ version` prints it.

use std::process::Command;

fn main() {
    let from_git = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let commit = from_git
        .or_else(|| {
            std::env::var("GITHUB_SHA")
                .ok()
                .map(|s| s.chars().take(12).collect())
        })
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=BUILD_COMMIT={commit}");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
}
