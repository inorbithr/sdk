//! The built-in templates, embedded from `templates/` by `build.rs`: one table for
//! `project init` and `ext init`, so there is one source and one way to read it.

include!(concat!(env!("OUT_DIR"), "/embedded.rs"));

/// The files under `prefix` (a folder, with its trailing slash), paths relative to it.
pub(crate) fn under(prefix: &str) -> impl Iterator<Item = (&'static str, &'static str)> {
    FILES
        .iter()
        .filter_map(move |(p, t)| p.strip_prefix(prefix).map(|rest| (rest, *t)))
}

/// One file, by its path under `templates/`.
pub(crate) fn file(path: &str) -> Option<&'static str> {
    FILES.iter().find(|(p, _)| *p == path).map(|(_, t)| *t)
}
