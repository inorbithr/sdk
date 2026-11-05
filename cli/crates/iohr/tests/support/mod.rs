//! Shared by the test files that run the binary (`mod support;`).

/// What a child process needs to run at all: `SYSTEMROOT` is required for sockets on
/// Windows (without it Winsock cannot load and a loopback connect fails with os error
/// 10106), the temp variables for downloads and scratch files. Nothing that could hold a
/// credential is passed on.
pub(crate) fn kept_env() -> Vec<(String, std::ffi::OsString)> {
    ["PATH", "SYSTEMROOT", "TEMP", "TMP", "TMPDIR"]
        .into_iter()
        .filter_map(|k| std::env::var_os(k).map(|v| (k.to_owned(), v)))
        .collect()
}
