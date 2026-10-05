//! `iohr sdk add`: the published SDK added to the project in the current directory, with
//! the package manager that project already uses (SR-31).
//!
//! What runs is decided here, from files alone ([`plan`], tested over temporary
//! directories), and then executed as one program with an argument vector: no shell, no
//! sudo, and nothing contacted by `iohr` itself. Only the package manager reaches its
//! registry, and it writes what it always writes (the manifest and its lock file).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::cli::{AddLang, SdkAdd};
use crate::error::Error;
use crate::output::Out;

/// The packages, as their manifests in this repository name them.
const CRATE: &str = "inorbithr";
const NPM: &str = "@inorbithr/sdk";
const JSR: &str = "jsr:@inorbithr/sdk";
const PYPI: &str = "inorbithr";
const GO_MODULE: &str = "github.com/inorbithr/sdk/go";

const DOCS: &str = "https://docs.inorbit.hr/docs/sdk/";

/// What `iohr sdk add` will run: `program args...` in `dir`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) lang: AddLang,
    /// The tool, as people know it: `cargo`, `pnpm`, `pip`.
    pub(crate) manager: &'static str,
    /// The program: a name looked up on `PATH`, or the active virtualenv's python.
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<String>,
    /// The project directory the command runs in.
    pub(crate) dir: PathBuf,
    /// Said after a successful run.
    pub(crate) notes: Vec<String>,
}

impl Plan {
    /// The command as one line a person could type.
    pub(crate) fn command_line(&self) -> String {
        std::iter::once(self.program.to_string_lossy().into_owned())
            .chain(self.args.iter().cloned())
            .map(|a| quote(&a))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// `iohr sdk add [LANG] [--version V] [--dry-run]`.
pub(crate) fn add(args: &SdkAdd, out: Out) -> Result<(), Error> {
    let cwd = std::env::current_dir()
        .map_err(|e| Error::Failed(format!("cannot read the current directory: {e}")))?;
    let lang = match args.lang {
        Some(lang) => lang,
        None => detect(&cwd)?,
    };
    let venv = std::env::var_os("VIRTUAL_ENV")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let plan = plan(lang, &cwd, args.version.as_deref(), venv.as_deref())?;
    let found = resolve(&plan.program);
    let command: Vec<String> = std::iter::once(plan.program.to_string_lossy().into_owned())
        .chain(plan.args.iter().cloned())
        .collect();
    if args.dry_run {
        if found.is_none() {
            Out::note(&format!("note: {}", missing(&plan)));
        }
        if out.json {
            Out::print_json(&serde_json::json!({
                "lang": name(plan.lang),
                "manager": plan.manager,
                "dir": plan.dir,
                "command": command,
                "dry_run": true,
                "found": found.is_some(),
            }));
        } else {
            Out::raw(format!("{}\n", plan.command_line()).as_bytes());
        }
        return Ok(());
    }
    let program = found.ok_or_else(|| Error::Failed(missing(&plan)))?;
    Out::note(&format!(
        "Running `{}` in {}",
        plan.command_line(),
        plan.dir.display()
    ));
    let mut child = Command::new(&program);
    child.args(&plan.args).current_dir(&plan.dir);
    if out.json {
        // stdout carries the JSON answer alone; the package manager talks on stderr.
        child.stdout(Stdio::from(std::io::stderr()));
    }
    let status = child
        .status()
        .map_err(|e| Error::Failed(format!("cannot run {}: {e}", program.display())))?;
    if !status.success() {
        return match status.code() {
            Some(code) => {
                Out::note(&format!("{} exited with {code}", plan.manager));
                Err(Error::Child(
                    u8::try_from(code).ok().filter(|c| *c != 0).unwrap_or(1),
                ))
            }
            None => Err(Error::Failed(format!(
                "{} was stopped by a signal",
                plan.manager
            ))),
        };
    }
    let example = example(plan.lang);
    if out.json {
        Out::print_json(&serde_json::json!({
            "lang": name(plan.lang),
            "manager": plan.manager,
            "dir": plan.dir,
            "command": command,
            "dry_run": false,
            "found": true,
            "notes": plan.notes,
            "example": example,
        }));
        return Ok(());
    }
    for note in &plan.notes {
        Out::note(&format!("note: {note}"));
    }
    Out::note(&format!(
        "\nAdded. A first call:\n\n{}\n\n`iohr sdk config` shows what a client will use here and where each value came from; more at {DOCS}",
        indent(example)
    ));
    Ok(())
}

/// The language of the nearest project between `start` and the repository root.
///
/// # Errors
///
/// [`Error::Usage`] when the nearest level holds files of several languages, or no level
/// holds any.
pub(crate) fn detect(start: &Path) -> Result<AddLang, Error> {
    for dir in ancestors(start) {
        let langs = languages_at(&dir);
        let mut each = langs.iter().copied();
        match (each.next(), each.next()) {
            (None, _) => {}
            (Some(lang), None) => return Ok(lang),
            (Some(_), Some(_)) => {
                let names: Vec<&str> = langs.iter().map(|l| name(*l)).collect();
                return Err(Error::Usage(format!(
                    "{} holds a {} project; say which one: {}",
                    dir.display(),
                    names.join(" and a "),
                    names
                        .iter()
                        .map(|n| format!("`iohr sdk add {n}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }
    }
    Err(Error::Usage(
        "no Cargo.toml, package.json, deno.json, pyproject.toml, requirements.txt or go.mod between here and the repository root; name the language: `iohr sdk add rust`, `iohr sdk add typescript`, `iohr sdk add python` or `iohr sdk add go`".into(),
    ))
}

/// What to run for `lang`, from the files between `start` and the repository root.
/// `virtual_env` is `VIRTUAL_ENV`, the one place pip may install into.
///
/// # Errors
///
/// [`Error::Usage`] when no project or manager can be chosen or the version is not one;
/// [`Error::Failed`] for a language that is not published yet.
pub(crate) fn plan(
    lang: AddLang,
    start: &Path,
    version: Option<&str>,
    virtual_env: Option<&Path>,
) -> Result<Plan, Error> {
    let version = version.map(check_version).transpose()?;
    let project = ancestors(start)
        .into_iter()
        .find(|d| languages_at(d).contains(&lang));
    let plan = |manager: &'static str, program: PathBuf, args: Vec<String>, dir: PathBuf| Plan {
        lang,
        manager,
        program,
        args,
        dir,
        notes: Vec::new(),
    };
    match lang {
        AddLang::Rust => {
            let dir = project.ok_or_else(|| {
                Error::Usage(
                    "no Cargo.toml between here and the repository root; make a crate with `cargo init` first".into(),
                )
            })?;
            let spec = version.map_or_else(|| CRATE.to_owned(), |v| format!("{CRATE}@{v}"));
            Ok(plan("cargo", "cargo".into(), vec!["add".into(), spec], dir))
        }
        AddLang::Go => {
            let dir = project.ok_or_else(|| {
                Error::Usage(
                    "no go.mod between here and the repository root; make a module with `go mod init <path>` first".into(),
                )
            })?;
            let at = version.map_or_else(|| "latest".to_owned(), |v| format!("v{v}"));
            Ok(plan(
                "go",
                "go".into(),
                vec!["get".into(), format!("{GO_MODULE}@{at}")],
                dir,
            ))
        }
        AddLang::Typescript => {
            let dir = project.unwrap_or_else(|| start.to_path_buf());
            let deno = (dir.join("deno.json").is_file() || dir.join("deno.jsonc").is_file())
                && !dir.join("package.json").is_file();
            if deno {
                let spec = version.map_or_else(|| JSR.to_owned(), |v| format!("{JSR}@{v}"));
                return Ok(plan("deno", "deno".into(), vec!["add".into(), spec], dir));
            }
            let manager = node_manager(&dir)?;
            let spec = version.map_or_else(|| NPM.to_owned(), |v| format!("{NPM}@{v}"));
            let verb = if manager == "npm" { "install" } else { "add" };
            Ok(plan(manager, manager.into(), vec![verb.into(), spec], dir))
        }
        AddLang::Python => {
            let dir = project.unwrap_or_else(|| start.to_path_buf());
            let spec = version.map_or_else(|| PYPI.to_owned(), |v| format!("{PYPI}=={v}"));
            if let Some(manager) = python_manager(&dir)? {
                return Ok(plan(manager, manager.into(), vec!["add".into(), spec], dir));
            }
            let Some(venv) = virtual_env else {
                return Err(Error::Usage(format!(
                    "no virtualenv is active and the project has no uv, poetry or pdm lock file; iohr does not install into a system Python. Either use uv:\n  uv init   # once, if there is no pyproject.toml\n  uv add {PYPI}\nor a virtualenv:\n  python3 -m venv .venv\n  . .venv/bin/activate   # .venv\\Scripts\\activate on Windows\n  iohr sdk add python"
                )));
            };
            let mut p = plan(
                "pip",
                venv_python(venv),
                vec!["-m".into(), "pip".into(), "install".into(), spec],
                dir.clone(),
            );
            if dir.join("pyproject.toml").is_file() {
                p.notes.push(format!(
                    "pip does not edit pyproject.toml: add \"{PYPI}\" to [project] dependencies"
                ));
            } else if dir.join("requirements.txt").is_file() {
                p.notes.push(format!(
                    "pip does not edit requirements.txt: add {PYPI} to it"
                ));
            }
            Ok(p)
        }
        AddLang::Csharp | AddLang::Java => Err(not_published(lang)),
    }
}

/// C# and Java build from source until their registry releases.
fn not_published(lang: AddLang) -> Error {
    let (what, registry, label) = if lang == AddLang::Csharp {
        ("InOrbit.Sdk", "NuGet", "C#")
    } else {
        ("hr.inorbit:inorbit-sdk", "Maven Central", "Java")
    };
    Error::Failed(format!(
        "the {label} SDK ({what}) is not published on {registry} yet; build it from source (https://github.com/inorbithr/sdk/tree/main/{}) or see {DOCS}",
        name(lang)
    ))
}

/// The JavaScript package manager: the nearest lock file between `dir` and the
/// repository root (a workspace keeps one at its root), or the `packageManager` field of
/// a `package.json` on the way; npm otherwise.
fn node_manager(dir: &Path) -> Result<&'static str, Error> {
    const LOCKS: &[(&str, &str)] = &[
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lock", "bun"),
        ("bun.lockb", "bun"),
        ("package-lock.json", "npm"),
        ("npm-shrinkwrap.json", "npm"),
    ];
    for level in ancestors(dir) {
        if let Some(m) = one_lock(&level, LOCKS)? {
            return Ok(m);
        }
        if let Some(m) = package_manager_field(&level.join("package.json")) {
            return Ok(m);
        }
    }
    Ok("npm")
}

/// uv, poetry or pdm, from the nearest lock file between `dir` and the repository root.
fn python_manager(dir: &Path) -> Result<Option<&'static str>, Error> {
    const LOCKS: &[(&str, &str)] = &[
        ("uv.lock", "uv"),
        ("poetry.lock", "poetry"),
        ("pdm.lock", "pdm"),
    ];
    for level in ancestors(dir) {
        if let Some(m) = one_lock(&level, LOCKS)? {
            return Ok(Some(m));
        }
    }
    Ok(None)
}

/// The manager whose lock file is in `dir`; two managers' lock files there are an error.
fn one_lock(dir: &Path, locks: &[(&str, &'static str)]) -> Result<Option<&'static str>, Error> {
    let present: Vec<(&str, &'static str)> = locks
        .iter()
        .copied()
        .filter(|(file, _)| dir.join(file).is_file())
        .collect();
    let managers: BTreeSet<&str> = present.iter().map(|(_, m)| *m).collect();
    match managers.len() {
        0 => Ok(None),
        1 => Ok(present.first().map(|(_, m)| *m)),
        _ => Err(Error::Usage(format!(
            "{} has lock files of several package managers ({}); remove the stale one, or run the manager you use yourself",
            dir.display(),
            present
                .iter()
                .map(|(f, _)| *f)
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// `"packageManager": "pnpm@9.12.0"` (Corepack) names the manager before any lock exists.
fn package_manager_field(package_json: &Path) -> Option<&'static str> {
    let text = std::fs::read_to_string(package_json).ok()?;
    let doc: serde_json::Value = serde_json::from_str(&text).ok()?;
    let field = doc.get("packageManager")?.as_str()?;
    let tool = field.split('@').next().unwrap_or_default();
    ["npm", "pnpm", "yarn", "bun"]
        .into_iter()
        .find(|m| *m == tool)
}

/// The python of a virtualenv.
fn venv_python(venv: &Path) -> PathBuf {
    if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    }
}

/// The project languages whose files are in `dir`.
fn languages_at(dir: &Path) -> BTreeSet<AddLang> {
    const MARKERS: &[(&str, AddLang)] = &[
        ("Cargo.toml", AddLang::Rust),
        ("package.json", AddLang::Typescript),
        ("deno.json", AddLang::Typescript),
        ("deno.jsonc", AddLang::Typescript),
        ("pyproject.toml", AddLang::Python),
        ("requirements.txt", AddLang::Python),
        ("uv.lock", AddLang::Python),
        ("poetry.lock", AddLang::Python),
        ("pdm.lock", AddLang::Python),
        ("go.mod", AddLang::Go),
    ];
    MARKERS
        .iter()
        .filter(|(file, _)| dir.join(file).is_file())
        .map(|(_, lang)| *lang)
        .collect()
}

/// `start` and its parents, up to and including the first that holds `.git` (a directory,
/// or the file a worktree or submodule has), or the filesystem root.
fn ancestors(start: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for dir in start.ancestors() {
        dirs.push(dir.to_path_buf());
        if dir.join(".git").exists() {
            break;
        }
    }
    dirs
}

/// A version as each manager takes it: digits first, letters, digits, `.`, `-` and `+`
/// only, at most 64 characters; a leading `v` is dropped (Go gets it back).
fn check_version(v: &str) -> Result<String, Error> {
    let bare = v.strip_prefix('v').unwrap_or(v);
    let ok = bare.len() <= 64
        && bare.chars().next().is_some_and(|c| c.is_ascii_digit())
        && bare
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    if ok {
        Ok(bare.to_owned())
    } else {
        Err(Error::Usage(format!(
            "--version takes a release such as 0.2.1; got {v:?}"
        )))
    }
}

/// The program's full path: itself when it is a path, otherwise the first match in an
/// absolute `PATH` entry. A relative entry (`.`, an empty one) is skipped, so a program
/// of that name in the project directory is never run.
fn resolve(program: &Path) -> Option<PathBuf> {
    if program.components().count() > 1 {
        return program.is_file().then(|| program.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .find_map(|dir| candidates(&dir, program).into_iter().find(|p| runnable(p)))
}

#[cfg(windows)]
fn candidates(dir: &Path, program: &Path) -> Vec<PathBuf> {
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    exts.split(';')
        .filter(|e| !e.is_empty())
        .map(|e| {
            let mut name = program.as_os_str().to_owned();
            name.push(e);
            dir.join(name)
        })
        .collect()
}

#[cfg(not(windows))]
fn candidates(dir: &Path, program: &Path) -> Vec<PathBuf> {
    vec![dir.join(program)]
}

#[cfg(unix)]
fn runnable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    p.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn runnable(p: &Path) -> bool {
    p.is_file()
}

/// Which tool is missing and where to get it.
fn missing(plan: &Plan) -> String {
    let how = match plan.manager {
        "cargo" => "install Rust with rustup: https://rustup.rs",
        "npm" => "it comes with Node.js: https://nodejs.org",
        "pnpm" => "`corepack enable pnpm`, or https://pnpm.io/installation",
        "yarn" => "`corepack enable yarn`, or https://yarnpkg.com/getting-started/install",
        "bun" => "https://bun.sh",
        "deno" => "https://deno.com",
        "uv" => "https://docs.astral.sh/uv/getting-started/installation/",
        "poetry" => "https://python-poetry.org/docs/#installation",
        "pdm" => "https://pdm-project.org/en/latest/#installation",
        "go" => "https://go.dev/doc/install",
        _ => {
            return format!(
                "the active virtualenv has no {}; make it again (`python3 -m venv .venv`) or deactivate it",
                plan.program.display()
            );
        }
    };
    format!("{} is not on PATH; {how}", plan.manager)
}

pub(crate) fn name(lang: AddLang) -> &'static str {
    match lang {
        AddLang::Rust => "rust",
        AddLang::Typescript => "typescript",
        AddLang::Python => "python",
        AddLang::Go => "go",
        AddLang::Csharp => "csharp",
        AddLang::Java => "java",
    }
}

/// A first call per language. Every line comes from that language's README (a test
/// holds them to it), so the snippet is the documented quick start.
fn example(lang: AddLang) -> &'static str {
    match lang {
        AddLang::Rust => {
            "use inorbithr::public::Surface as _;\nuse inorbithr::{Client, Error};\n\n// in an async fn returning Result<(), Error>\nlet client: Client = Client::builder()\n    .load()?;\nlet me = client.me().await?;"
        }
        AddLang::Typescript => {
            "import { Public } from \"@inorbithr/sdk\";\n\n// The environment, the iohr config file and the iohr login, in that order.\nconst api = Public.load();\nconst { value: me } = await api.me();"
        }
        AddLang::Python => {
            "from inorbithr import Public\n\napi = Public.load()  # code, then INORBIT_*, then the iohr config file, then `iohr login`\nme = api.me().value"
        }
        AddLang::Go => {
            "import (\n\t\"github.com/inorbithr/sdk/go/public\"\n)\n\n// The environment, the iohr config file and the iohr login, in that order.\napi, err := public.Load(ctx)\nif err != nil {\n\treturn err\n}\nme, err := api.Me(ctx)"
        }
        AddLang::Csharp | AddLang::Java => "",
    }
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("    {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// An argument as a shell would need it to read it back; only for display.
fn quote(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c));
    if plain {
        arg.to_owned()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests fail by panicking")]

    use std::path::{Path, PathBuf};

    use super::{AddLang, Plan, detect, example, plan};
    use crate::error::Error;

    /// A repository root (it holds `.git`) with `files` in it, paths relative to it.
    fn repo(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        for f in files {
            let p = dir.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        dir
    }

    fn argv(p: &Plan) -> Vec<String> {
        std::iter::once(p.program.to_string_lossy().into_owned())
            .chain(p.args.iter().cloned())
            .collect()
    }

    fn run(lang: AddLang, dir: &Path, version: Option<&str>) -> Vec<String> {
        argv(&plan(lang, dir, version, None).unwrap())
    }

    fn usage(r: Result<impl std::fmt::Debug, Error>) -> String {
        match r {
            Err(Error::Usage(m)) => m,
            other => panic!("expected a usage error, got {other:?}"),
        }
    }

    #[test]
    fn detects_each_language_from_its_files() {
        for (file, lang) in [
            ("Cargo.toml", AddLang::Rust),
            ("package.json", AddLang::Typescript),
            ("deno.json", AddLang::Typescript),
            ("pyproject.toml", AddLang::Python),
            ("requirements.txt", AddLang::Python),
            ("uv.lock", AddLang::Python),
            ("poetry.lock", AddLang::Python),
            ("go.mod", AddLang::Go),
        ] {
            let r = repo(&[file]);
            assert_eq!(detect(r.path()).unwrap(), lang, "{file}");
        }
    }

    #[test]
    fn the_nearest_level_wins_and_the_command_runs_there() {
        let r = repo(&["Cargo.toml", "web/package.json", "web/src/x.ts"]);
        let start = r.path().join("web/src");
        assert_eq!(detect(&start).unwrap(), AddLang::Typescript);
        let p = plan(AddLang::Typescript, &start, None, None).unwrap();
        assert_eq!(p.dir, r.path().join("web"));
        assert_eq!(detect(r.path()).unwrap(), AddLang::Rust);
    }

    #[test]
    fn several_languages_at_one_level_ask_for_one() {
        let r = repo(&["Cargo.toml", "package.json"]);
        let m = usage(detect(r.path()));
        assert!(m.contains("`iohr sdk add rust`"), "{m}");
        assert!(m.contains("`iohr sdk add typescript`"), "{m}");
    }

    #[test]
    fn no_project_names_the_four_commands() {
        let r = repo(&[]);
        let m = usage(detect(r.path()));
        for lang in ["rust", "typescript", "python", "go"] {
            assert!(m.contains(&format!("`iohr sdk add {lang}`")), "{m}");
        }
    }

    #[test]
    fn the_walk_stops_at_the_repository_root() {
        // A Cargo.toml above the repository is someone else's project.
        let outer = tempfile::tempdir().unwrap();
        std::fs::write(outer.path().join("Cargo.toml"), "").unwrap();
        let inner = outer.path().join("repo");
        std::fs::create_dir_all(inner.join(".git")).unwrap();
        usage(detect(&inner));
        // A worktree or submodule has a .git file, which stops it too.
        let wt = outer.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), "gitdir: elsewhere\n").unwrap();
        usage(detect(&wt));
    }

    #[test]
    fn each_javascript_manager_from_its_lock_file() {
        for (lock, want) in [
            ("pnpm-lock.yaml", vec!["pnpm", "add", "@inorbithr/sdk"]),
            ("yarn.lock", vec!["yarn", "add", "@inorbithr/sdk"]),
            ("bun.lock", vec!["bun", "add", "@inorbithr/sdk"]),
            ("bun.lockb", vec!["bun", "add", "@inorbithr/sdk"]),
            (
                "package-lock.json",
                vec!["npm", "install", "@inorbithr/sdk"],
            ),
        ] {
            let r = repo(&["package.json", lock]);
            assert_eq!(run(AddLang::Typescript, r.path(), None), want, "{lock}");
        }
        let r = repo(&["package.json"]);
        assert_eq!(
            run(AddLang::Typescript, r.path(), Some("0.2.1")),
            ["npm", "install", "@inorbithr/sdk@0.2.1"]
        );
        let r = repo(&["pnpm-lock.yaml", "package.json"]);
        assert_eq!(
            run(AddLang::Typescript, r.path(), Some("v0.2.1")),
            ["pnpm", "add", "@inorbithr/sdk@0.2.1"]
        );
    }

    #[test]
    fn deno_without_package_json_adds_from_jsr() {
        let r = repo(&["deno.json"]);
        assert_eq!(
            run(AddLang::Typescript, r.path(), None),
            ["deno", "add", "jsr:@inorbithr/sdk"]
        );
        let r = repo(&["deno.jsonc"]);
        assert_eq!(
            run(AddLang::Typescript, r.path(), Some("0.2.1")),
            ["deno", "add", "jsr:@inorbithr/sdk@0.2.1"]
        );
        // With a package.json beside it, the Node manager decides.
        let r = repo(&["deno.json", "package.json"]);
        assert_eq!(run(AddLang::Typescript, r.path(), None)[0], "npm");
    }

    #[test]
    fn a_workspace_member_uses_the_lock_at_the_root() {
        let r = repo(&[
            "package.json",
            "pnpm-lock.yaml",
            "packages/app/package.json",
        ]);
        let p = plan(
            AddLang::Typescript,
            &r.path().join("packages/app"),
            None,
            None,
        )
        .unwrap();
        assert_eq!(argv(&p), ["pnpm", "add", "@inorbithr/sdk"]);
        assert_eq!(p.dir, r.path().join("packages/app"));
        let r = repo(&["pyproject.toml", "uv.lock", "libs/a/pyproject.toml"]);
        assert_eq!(
            run(AddLang::Python, &r.path().join("libs/a"), None),
            ["uv", "add", "inorbithr"]
        );
    }

    #[test]
    fn the_package_manager_field_decides_before_a_lock_exists() {
        let r = repo(&[]);
        std::fs::write(
            r.path().join("package.json"),
            r#"{"name":"x","packageManager":"yarn@4.5.0+sha512.abc"}"#,
        )
        .unwrap();
        assert_eq!(
            run(AddLang::Typescript, r.path(), None),
            ["yarn", "add", "@inorbithr/sdk"]
        );
    }

    #[test]
    fn two_managers_lock_files_are_refused() {
        let r = repo(&["package.json", "pnpm-lock.yaml", "yarn.lock"]);
        let m = usage(plan(AddLang::Typescript, r.path(), None, None));
        assert!(
            m.contains("pnpm-lock.yaml") && m.contains("yarn.lock"),
            "{m}"
        );
        let r = repo(&["pyproject.toml", "uv.lock", "poetry.lock"]);
        usage(plan(AddLang::Python, r.path(), None, None));
    }

    #[test]
    fn each_python_manager_from_its_lock_file() {
        for (lock, tool) in [
            ("uv.lock", "uv"),
            ("poetry.lock", "poetry"),
            ("pdm.lock", "pdm"),
        ] {
            let r = repo(&["pyproject.toml", lock]);
            assert_eq!(
                run(AddLang::Python, r.path(), None),
                [tool, "add", "inorbithr"]
            );
            assert_eq!(
                run(AddLang::Python, r.path(), Some("0.2.1")),
                [tool, "add", "inorbithr==0.2.1"]
            );
        }
    }

    #[test]
    fn pip_only_into_the_active_virtualenv() {
        let r = repo(&["requirements.txt"]);
        let m = usage(plan(AddLang::Python, r.path(), None, None));
        assert!(m.contains("uv add inorbithr"), "{m}");
        assert!(m.contains("python3 -m venv .venv"), "{m}");
        let venv = PathBuf::from("/work/.venv");
        let p = plan(AddLang::Python, r.path(), Some("0.2.1"), Some(&venv)).unwrap();
        let python = if cfg!(windows) {
            venv.join("Scripts").join("python.exe")
        } else {
            venv.join("bin").join("python")
        };
        assert_eq!(p.program, python);
        assert_eq!(p.args, ["-m", "pip", "install", "inorbithr==0.2.1"]);
        assert_eq!(p.manager, "pip");
        assert!(p.notes[0].contains("requirements.txt"), "{:?}", p.notes);
        // A lock file wins over the virtualenv.
        let r = repo(&["pyproject.toml", "uv.lock"]);
        let p = plan(AddLang::Python, r.path(), None, Some(&venv)).unwrap();
        assert_eq!(argv(&p), ["uv", "add", "inorbithr"]);
    }

    #[test]
    fn rust_and_go_need_their_manifest() {
        let r = repo(&["Cargo.toml"]);
        assert_eq!(
            run(AddLang::Rust, r.path(), None),
            ["cargo", "add", "inorbithr"]
        );
        assert_eq!(
            run(AddLang::Rust, r.path(), Some("0.2.1")),
            ["cargo", "add", "inorbithr@0.2.1"]
        );
        let r = repo(&["go.mod"]);
        assert_eq!(
            run(AddLang::Go, r.path(), None),
            ["go", "get", "github.com/inorbithr/sdk/go@latest"]
        );
        for v in ["0.2.1", "v0.2.1"] {
            assert_eq!(
                run(AddLang::Go, r.path(), Some(v)),
                ["go", "get", "github.com/inorbithr/sdk/go@v0.2.1"]
            );
        }
        let empty = repo(&[]);
        assert!(usage(plan(AddLang::Rust, empty.path(), None, None)).contains("cargo init"));
        assert!(usage(plan(AddLang::Go, empty.path(), None, None)).contains("go mod init"));
    }

    #[test]
    fn csharp_and_java_are_not_published_yet() {
        let r = repo(&[]);
        for lang in [AddLang::Csharp, AddLang::Java] {
            match plan(lang, r.path(), None, None) {
                Err(e @ Error::Failed(_)) => {
                    assert!(e.to_string().contains("not published"), "{e}");
                    assert_eq!(e.exit_code(), std::process::ExitCode::from(1));
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_version_that_is_not_one_is_refused() {
        let r = repo(&["Cargo.toml"]);
        for v in ["", "-x", "--git=evil", "1 2", "1;2", "latest", "0.2.1/../x"] {
            usage(plan(AddLang::Rust, r.path(), Some(v), None));
        }
        for v in ["0.2.1", "1.0.0-rc.1", "0.3.0a1"] {
            plan(AddLang::Rust, r.path(), Some(v), None).unwrap();
        }
    }

    #[test]
    fn the_command_line_quotes_what_a_shell_would_split() {
        let p = Plan {
            lang: AddLang::Python,
            manager: "pip",
            program: PathBuf::from("/home/a b/.venv/bin/python"),
            args: vec![
                "-m".into(),
                "pip".into(),
                "install".into(),
                "inorbithr==0.2.1".into(),
            ],
            dir: PathBuf::from("/"),
            notes: Vec::new(),
        };
        assert_eq!(
            p.command_line(),
            "'/home/a b/.venv/bin/python' -m pip install inorbithr==0.2.1"
        );
    }

    #[test]
    fn every_example_line_is_in_the_language_readme() {
        for (lang, readme) in [
            (AddLang::Rust, include_str!("../../../../../rust/README.md")),
            (
                AddLang::Typescript,
                include_str!("../../../../../typescript/README.md"),
            ),
            (
                AddLang::Python,
                include_str!("../../../../../python/README.md"),
            ),
            (AddLang::Go, include_str!("../../../../../go/README.md")),
        ] {
            for line in example(lang).lines().map(str::trim) {
                if line.is_empty() || line.starts_with("//") {
                    continue;
                }
                assert!(
                    readme.contains(line),
                    "{lang:?}: `{line}` is not in the README's quick start any more"
                );
            }
        }
    }
}
