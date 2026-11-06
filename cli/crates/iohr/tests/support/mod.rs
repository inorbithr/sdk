//! Shared by the test files that run the binary (`mod support;`).

/// What a child process needs to run at all: `SYSTEMROOT` is required for sockets on
/// Windows (without it Winsock cannot load and a loopback connect fails with os error
/// 10106), the temp variables for downloads and scratch files. Nothing that could hold a
/// credential is passed on. `IOHR_NO_BROWSER` is always set: a test never opens a real
/// tab on the developer's machine.
pub(crate) fn kept_env() -> Vec<(String, std::ffi::OsString)> {
    ["PATH", "SYSTEMROOT", "TEMP", "TMP", "TMPDIR"]
        .into_iter()
        .filter_map(|k| std::env::var_os(k).map(|v| (k.to_owned(), v)))
        .chain([("IOHR_NO_BROWSER".to_owned(), "1".into())])
        .collect()
}

/// The folder's real name, without the `\\?\` prefix Windows `canonicalize` adds: what the
/// binary prints for a folder.
#[allow(dead_code)] // used by some of the files that share this module
pub(crate) fn canon(p: &std::path::Path) -> std::path::PathBuf {
    let c = std::fs::canonicalize(p).unwrap();
    match c.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with("UNC") => rest.into(),
        _ => c,
    }
}
