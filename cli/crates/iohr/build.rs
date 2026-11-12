//! Embeds the built-in templates: every file under `templates/` goes into one table, so a
//! template is a folder and adding one needs no list to keep in step (`src/embedded.rs`).
//!
//! A path component `dot-X` is written as `.X` (a template folder cannot hold a live
//! `.github/`), one named `gitignore` as `.gitignore`, and a file ending in `.tmpl` without
//! it (`Cargo.toml.tmpl`: a live `Cargo.toml` in a template folder is a package of its own).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a build script fails the build by panicking"
)]

use std::fmt::Write as _;
use std::path::Path;

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .expect("templates folder")
        .map(|e| e.expect("entry"))
        .collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for e in entries {
        let p = e.path();
        if e.file_type().expect("file type").is_dir() {
            walk(root, &p, out);
        } else {
            let rel = p.strip_prefix(root).expect("inside root");
            let path = rel
                .components()
                .map(|c| {
                    let s = c.as_os_str().to_str().expect("utf-8 path");
                    match s.strip_prefix("dot-") {
                        Some(rest) => format!(".{rest}"),
                        None if s == "gitignore" => ".gitignore".to_owned(),
                        None => s.to_owned(),
                    }
                })
                .collect::<Vec<_>>()
                .join("/");
            let path = path
                .strip_suffix(".tmpl")
                .map_or(path.clone(), str::to_owned);
            out.push((path, p.to_str().expect("utf-8 path").to_owned()));
        }
    }
}

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it");
    let root = Path::new(&manifest).join("templates");
    println!("cargo:rerun-if-changed=templates");
    let mut files = Vec::new();
    walk(&root, &root, &mut files);
    let mut src = String::from("pub(crate) const FILES: &[(&str, &str)] = &[\n");
    for (path, abs) in &files {
        println!("cargo:rerun-if-changed={abs}");
        writeln!(src, "    ({path:?}, include_str!({abs:?})),").unwrap();
    }
    src.push_str("];\n");
    let out = std::env::var("OUT_DIR").expect("cargo sets it");
    std::fs::write(Path::new(&out).join("embedded.rs"), src).expect("write embedded.rs");
}
