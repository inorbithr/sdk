//! `iohr ext init PUBLISHER/NAME --lang rust|python|shell`: a folder with everything an
//! extension needs (PRD 0084): the manifest with every field present and commented, an
//! entry point that asks the token channel for a token and writes a heartbeat, a
//! self-test, a README and a release workflow that signs keylessly and attaches the
//! bundles with `oras attach`.

use std::path::{Path, PathBuf};

use super::lint::{contains_tbd, lint};
use super::manifest::{Lang, ManifestError, SOURCE_FILE, split_id};

/// One file of a template: where it goes (placeholders allowed), its text, and whether
/// it is a program.
struct File {
    path: &'static str,
    text: &'static str,
    program: bool,
}

macro_rules! file {
    ($path:literal, $src:literal) => {
        File {
            path: $path,
            text: include_str!(concat!("../../templates/ext/", $src)),
            program: false,
        }
    };
    ($path:literal, $src:literal, program) => {
        File {
            path: $path,
            text: include_str!(concat!("../../templates/ext/", $src)),
            program: true,
        }
    };
}

const COMMON: &[File] = &[
    file!("extension.toml", "common/extension.toml"),
    file!("README.md", "common/README.md"),
    file!(".gitignore", "common/gitignore"),
    file!(
        ".github/workflows/release.yml",
        "common/workflows/release.yml"
    ),
    file!(
        "tools/ext_artifact.py",
        "common/tools/ext_artifact.py",
        program
    ),
];

const SHELL: &[File] = &[
    file!("bin/@@NAME@@", "shell/bin/entry", program),
    file!("tests/selftest.sh", "shell/tests/selftest.sh", program),
];

const PYTHON: &[File] = &[
    file!("pyproject.toml", "python/pyproject.toml"),
    file!(
        "src/@@MODULE@@/__init__.py",
        "python/src/module/__init__.py"
    ),
    file!(
        "src/@@MODULE@@/__main__.py",
        "python/src/module/__main__.py"
    ),
    file!("src/@@MODULE@@/channel.py", "python/src/module/channel.py"),
    file!("tests/test_selftest.py", "python/tests/test_selftest.py"),
];

const RUST: &[File] = &[
    file!("Cargo.toml", "rust/Cargo.toml.tmpl"),
    file!("src/main.rs", "rust/src/main.rs"),
    file!("tests/selftest.rs", "rust/tests/selftest.rs"),
];

const SCRIPT_MATRIX: &str = "          - { os: ubuntu-24.04, dir: any }";

const RUST_MATRIX: &str =
    "          - { os: ubuntu-24.04, target: x86_64-unknown-linux-musl, dir: linux-amd64 }
          - { os: ubuntu-24.04-arm, target: aarch64-unknown-linux-musl, dir: linux-arm64 }
          - { os: macos-15, target: aarch64-apple-darwin, dir: darwin-arm64 }";

const SHELL_BUILD: &str = r#"      - name: Build (the script, for every platform it targets)
        run: |
          for p in $(python3 -c 'import tomllib; print(" ".join(tomllib.load(open("extension.toml", "rb"))["targets"]["platforms"]))'); do
            mkdir -p "dist/bin/${p/\//-}"
            install -m 0755 "bin/$NAME" "dist/bin/${p/\//-}/$NAME"
          done"#;

const PYTHON_BUILD: &str = r#"      - name: Build (one zipapp, for every platform it targets)
        run: |
          module="${NAME//-/_}"
          python3 -m zipapp src -m "$module.__main__:main" -p "/usr/bin/env python3" -o "$RUNNER_TEMP/$NAME"
          for p in $(python3 -c 'import tomllib; print(" ".join(tomllib.load(open("extension.toml", "rb"))["targets"]["platforms"]))'); do
            mkdir -p "dist/bin/${p/\//-}"
            install -m 0755 "$RUNNER_TEMP/$NAME" "dist/bin/${p/\//-}/$NAME"
          done"#;

const RUST_BUILD: &str = r#"      - name: Build
        env:
          TARGET: ${{ matrix.target }}
          DIR: ${{ matrix.dir }}
        run: |
          if [[ "$TARGET" == *musl ]]; then sudo apt-get update && sudo apt-get install -y musl-tools; fi
          rustup target add "$TARGET"
          cargo build --release --locked --target "$TARGET"
          mkdir -p "dist/bin/$DIR"
          install -m 0755 "target/$TARGET/release/$NAME" "dist/bin/$DIR/$NAME""#;

const SCRIPT_NOTES: &str = "## Layout\n\n\
`extension.toml` the manifest; `bin/@@NAME@@` the program, a POSIX shell script that \
needs `curl` for the token channel; `tests/selftest.sh` the self-test. Nothing to build: \
`iohr ext dev` runs the script as it is.";

const PYTHON_NOTES: &str = "## Layout\n\n\
`extension.toml` the manifest; `src/@@MODULE@@/` the package (`channel.py` speaks iohr's \
token channel with the standard library alone); `tests/` the self-test, run with \
`python3 -m unittest`. `iohr ext dev` runs `python3 -m @@MODULE@@` with `src` on \
`PYTHONPATH`; the release ships it as one zipapp.";

const RUST_NOTES: &str = "## Layout\n\n\
`extension.toml` the manifest; `src/main.rs` the program, on the `inorbithr` crate, whose \
`ExtSocketToken` asks iohr's token channel (and `Client::load` uses that channel alone \
when iohr runs the program); `tests/selftest.rs` the self-test. `iohr ext dev` runs \
`cargo run`, `iohr ext dev --test` runs `cargo test`.";

/// What `init` wrote.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Scaffold {
    /// The folder.
    pub dir: PathBuf,
    /// The files, relative to it.
    pub files: Vec<String>,
}

/// Why nothing was written.
#[derive(Debug, thiserror::Error)]
pub enum InitError {
    /// The name or the folder cannot be used.
    #[error("{0}")]
    Usage(String),
    /// Writing failed.
    #[error("{0}")]
    Io(String),
}

impl From<ManifestError> for InitError {
    fn from(e: ManifestError) -> Self {
        Self::Usage(e.0)
    }
}

/// The placeholders of one scaffold.
fn values(id: &str, name: &str, lang: Lang) -> Vec<(&'static str, String)> {
    let (platforms, needs, matrix, build, notes) = match lang {
        Lang::Shell => (
            r#""darwin/arm64", "linux/amd64", "linux/arm64""#,
            r#""sh", "curl""#,
            SCRIPT_MATRIX,
            SHELL_BUILD,
            SCRIPT_NOTES,
        ),
        Lang::Python => (
            r#""darwin/arm64", "linux/amd64", "linux/arm64""#,
            r#""python3""#,
            SCRIPT_MATRIX,
            PYTHON_BUILD,
            PYTHON_NOTES,
        ),
        Lang::Rust => (
            r#""darwin/arm64", "linux/amd64", "linux/arm64""#,
            "",
            RUST_MATRIX,
            RUST_BUILD,
            RUST_NOTES,
        ),
    };
    vec![
        // Longer texts first: they hold placeholders of their own.
        ("@@LANG_NOTES@@", notes.to_owned()),
        ("@@MATRIX@@", matrix.to_owned()),
        ("@@BUILD@@", build.to_owned()),
        ("@@ID@@", id.to_owned()),
        ("@@NAME@@", name.to_owned()),
        ("@@MODULE@@", name.replace('-', "_")),
        ("@@ENTRYPOINT@@", name.to_owned()),
        ("@@LANG@@", lang.as_str().to_owned()),
        ("@@PLATFORMS@@", platforms.to_owned()),
        ("@@NEEDS@@", needs.to_owned()),
        ("@@INORBITHR@@", inorbithr::VERSION.to_owned()),
        ("@@IOHR_VERSION@@", env!("CARGO_PKG_VERSION").to_owned()),
    ]
}

fn fill(text: &str, values: &[(&str, String)]) -> String {
    values
        .iter()
        .fold(text.to_owned(), |t, (k, v)| t.replace(k, v))
}

/// The files of a scaffold, in memory: relative path, text, whether it is a program.
///
/// # Errors
///
/// [`InitError::Usage`] for an id that is not `publisher/name` or holds `tbd`.
pub fn render(id: &str, lang: Lang) -> Result<Vec<(String, String, bool)>, InitError> {
    if contains_tbd(id) {
        return Err(InitError::Usage(format!(
            "`{id}` holds \"tbd\": name the extension for what it does before it is built"
        )));
    }
    let (_, name) = split_id(id)?;
    let values = values(id, name, lang);
    let lang_files = match lang {
        Lang::Shell => SHELL,
        Lang::Python => PYTHON,
        Lang::Rust => RUST,
    };
    Ok(COMMON
        .iter()
        .chain(lang_files)
        .map(|f| (fill(f.path, &values), fill(f.text, &values), f.program))
        .collect())
}

/// Writes the scaffold of `id` in `lang` into `dir` (default `./NAME`), which must not
/// exist or be empty. The manifest written must pass the lint.
///
/// # Errors
///
/// [`InitError`] when the id or the folder cannot be used, or writing fails.
pub fn init(id: &str, lang: Lang, dir: Option<&Path>) -> Result<Scaffold, InitError> {
    let files = render(id, lang)?;
    let (_, name) = split_id(id)?;
    let dir = dir.map_or_else(|| PathBuf::from(name), Path::to_path_buf);
    if dir.exists()
        && std::fs::read_dir(&dir)
            .map_err(|e| InitError::Io(format!("cannot read {}: {e}", dir.display())))?
            .next()
            .is_some()
    {
        return Err(InitError::Usage(format!(
            "{} is not empty; choose another folder with --dir",
            dir.display()
        )));
    }
    let manifest = files
        .iter()
        .find(|(p, _, _)| p == SOURCE_FILE)
        .map(|(_, t, _)| t.as_str())
        .unwrap_or_default();
    let report = lint(manifest);
    if !report.ok {
        return Err(InitError::Io(format!(
            "the template's manifest does not pass the lint: {}",
            report
                .findings
                .iter()
                .map(|f| f.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }
    for (rel, text, program) in &files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| InitError::Io(format!("cannot create {}: {e}", parent.display())))?;
        }
        std::fs::write(&path, text)
            .map_err(|e| InitError::Io(format!("cannot write {}: {e}", path.display())))?;
        #[cfg(unix)]
        if *program {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).map_err(
                |e| InitError::Io(format!("cannot make {} a program: {e}", path.display())),
            )?;
        }
        #[cfg(not(unix))]
        let _ = program;
    }
    Ok(Scaffold {
        dir,
        files: files.into_iter().map(|(p, _, _)| p).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::{Lang, init, render};
    use crate::ext::lint::lint;

    #[test]
    fn every_language_renders_a_manifest_that_passes_and_no_placeholder_is_left() {
        for lang in [Lang::Shell, Lang::Python, Lang::Rust] {
            let files = render("inorbit/hello-world", lang).unwrap();
            for (path, text, _) in &files {
                assert!(!path.contains("@@"), "{path}");
                assert!(!text.contains("@@"), "{lang:?} {path}: {text}");
            }
            let (_, manifest, _) = files
                .iter()
                .find(|(p, _, _)| p == "extension.toml")
                .unwrap();
            let r = lint(manifest);
            assert!(r.ok, "{lang:?}: {:?}", r.findings);
            assert!(manifest.contains(&format!("lang = \"{}\"", lang.as_str())));
            // Every field is present: in the source, commented or set.
            for key in [
                "[runs]",
                "[permissions]",
                "scopes",
                "privileges",
                "network",
                "exec",
                "[permissions.filesystem]",
                "[permissions.data]",
                "[resources]",
                "[health]",
                "secrets",
                "[targets]",
                "[upgrade]",
                "migrate",
                "rollback",
                "check",
            ] {
                assert!(manifest.contains(key), "{lang:?}: {key}");
            }
        }
    }

    #[test]
    fn tbd_and_bad_ids_are_refused_and_nothing_is_written() {
        let dir = tempfile::tempdir().unwrap();
        for bad in [
            "inorbit/tbd",
            "inorbit/hello-TBD",
            "TBD/hello",
            "hello",
            "inorbit/ext",
        ] {
            let out = dir.path().join("x");
            assert!(init(bad, Lang::Shell, Some(&out)).is_err(), "{bad}");
            assert!(!out.exists(), "{bad}");
        }
    }

    #[test]
    fn a_folder_with_files_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mine.txt"), "keep").unwrap();
        let e = init("inorbit/hello", Lang::Shell, Some(dir.path())).unwrap_err();
        assert!(e.to_string().contains("not empty"), "{e}");
    }
}
