//! The conformance vectors (`conformance/vectors/`, `docs/config.md` section 9.2) run
//! against the resolver: `config`, `config-path` and `durations`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test helpers fail the test by panicking"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::{Inputs, Os, config_path, parse_duration, resolve};

fn vectors(kind: &str) -> Vec<(PathBuf, Value)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../conformance/vectors")
        .join(kind);
    let mut out: Vec<(PathBuf, Value)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
        .map(|p| {
            let text = std::fs::read_to_string(&p).unwrap();
            let v: Value = serde_yaml_ng::from_str(&text).unwrap();
            (p, v)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(!out.is_empty(), "no vectors in {}", dir.display());
    out
}

fn os_of(v: &Value) -> Os {
    match v.as_str() {
        Some("macos") => Os::Macos,
        Some("windows") => Os::Windows,
        _ => Os::Linux,
    }
}

fn env_of(v: &Value, dir: &str) -> BTreeMap<String, String> {
    v.as_object()
        .map(|m| {
            m.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        v.as_str().unwrap_or_default().replace("{dir}", dir),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `expected` is a subset of `actual`: objects by key, arrays element by element with
/// the same length, scalars equal.
fn subset(expected: &Value, actual: &Value, at: &str) -> Result<(), String> {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for (k, ev) in e {
                let av = a
                    .get(k)
                    .ok_or_else(|| format!("{at}.{k}: missing in {actual}"))?;
                subset(ev, av, &format!("{at}.{k}"))?;
            }
            Ok(())
        }
        (Value::Array(e), Value::Array(a)) => {
            if e.len() != a.len() {
                return Err(format!("{at}: expected {expected}, got {actual}"));
            }
            for (i, (ev, av)) in e.iter().zip(a).enumerate() {
                subset(ev, av, &format!("{at}[{i}]"))?;
            }
            Ok(())
        }
        (e, a) if e == a => Ok(()),
        // Paths compare after `\` becomes `/` (vector.schema.json).
        (Value::String(e), Value::String(a)) if e.replace('\\', "/") == a.replace('\\', "/") => {
            Ok(())
        }
        (e, a) => Err(format!("{at}: expected {e}, got {a}")),
    }
}

fn substitute(v: &Value, dir: &str, file: &str) -> Value {
    match v {
        Value::String(s) => Value::String(s.replace("{file}", file).replace("{dir}", dir)),
        Value::Array(a) => Value::Array(a.iter().map(|x| substitute(x, dir, file)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| (k.clone(), substitute(x, dir, file)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[allow(clippy::too_many_lines, reason = "one vector, every expectation")]
fn run_config_vector(path: &Path, v: &Value) -> Result<(), String> {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_str().unwrap().to_owned();
    let input = v.get("input").cloned().unwrap_or_else(|| json!({}));
    let os = os_of(&input["os"]);
    let env = env_of(&input["env"], &dir);
    let home_flag = input["home"].as_bool().unwrap_or(false);
    let home = home_flag.then(|| format!("{dir}/home"));
    if let Some(h) = &home {
        std::fs::create_dir_all(h).unwrap();
    }
    if let Some(files) = input["files"].as_object() {
        for (name, content) in files {
            let p = tmp.path().join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content.as_str().unwrap_or_default()).unwrap();
        }
    }
    let mut code: Map<String, Value> = input["code"].as_object().cloned().unwrap_or_default();
    let mut file = format!("{dir}/config.toml");
    if let Some(text) = input["config_file"].as_str() {
        if home_flag {
            // Where the file would be read from, INORBIT_CONFIG_FILE aside: `off` still
            // finds a file there and must not read it.
            let mut located = env.clone();
            located.remove("INORBIT_CONFIG_FILE");
            let (p, _, _) = config_path(os, &located, home.as_deref(), None)
                .ok_or("the vector writes a file at a default location that does not exist")?;
            std::fs::create_dir_all(Path::new(&p).parent().unwrap()).unwrap();
            std::fs::write(&p, text).unwrap();
            file = p;
        } else {
            std::fs::write(&file, text).unwrap();
            code.insert("config_file".into(), json!(file));
        }
    }
    let cli_present = input["cli"].as_str() == Some("present");
    let cli_found = move |_: &str| cli_present;
    let read = |p: &str| std::fs::read(p).ok();
    let inputs = Inputs {
        env: &env,
        os,
        home,
        cwd: dir.clone(),
        code,
        profile_type: input["profile_type"].as_str().map(str::to_owned),
        cli_found: &cli_found,
        read: &read,
    };
    let expect = substitute(&v["expect"], &dir, &file);
    let got = resolve(&inputs);
    let name = path.file_name().unwrap().to_string_lossy();
    let shown = match &got {
        Ok(d) => d.to_string(),
        Err(e) => format!("{e} {:?}", e.problems),
    };
    if let Some(ex) = expect["excludes"].as_array() {
        for x in ex {
            let x = x.as_str().unwrap_or_default();
            if shown.contains(x) {
                return Err(format!("{name}: {x:?} appears in {shown}"));
            }
        }
    }
    if let Some(err) = expect.get("error") {
        let e = match got {
            Ok(d) => return Err(format!("{name}: expected an error, got {d}")),
            Err(e) => e,
        };
        let text = e.to_string();
        if let Some(problems) = err["problems"].as_array() {
            if problems.len() != e.problems.len() {
                return Err(format!(
                    "{name}: expected {} problems, got {:?}",
                    problems.len(),
                    e.problems
                ));
            }
            for (want, have) in problems.iter().zip(&e.problems) {
                if let Some(s) = want["setting"].as_str()
                    && s != have.setting
                {
                    return Err(format!("{name}: setting {s} != {have:?}"));
                }
                if let Some(s) = want["source"].as_str()
                    && s != have.source
                {
                    return Err(format!("{name}: source {s:?} != {have:?}"));
                }
                if let Some(s) = want["message_contains"].as_str()
                    && !have.message.contains(s)
                {
                    return Err(format!("{name}: message lacks {s:?}: {have:?}"));
                }
            }
        }
        if let Some(parts) = err["message_contains"].as_array() {
            for p in parts {
                let p = p.as_str().unwrap_or_default();
                if !text.contains(p) {
                    return Err(format!("{name}: error lacks {p:?}:\n{text}"));
                }
            }
        }
        return Ok(());
    }
    let d = got.map_err(|e| format!("{name}: unexpected error:\n{e}"))?;
    for key in ["profile", "settings", "credential", "pipeline"] {
        if let Some(want) = expect.get(key) {
            subset(want, &d[key], &format!("{name}: {key}"))?;
        }
    }
    if let Some(want) = expect.get("config_file") {
        let have = d["config_file"].as_str().map(|s| s.replace('\\', "/"));
        let want = want.as_str().map(|s| s.replace('\\', "/"));
        if want != have {
            return Err(format!("{name}: config_file {want:?} != {have:?}"));
        }
    }
    if let Some(absent) = expect["settings_absent"].as_array() {
        for a in absent {
            let a = a.as_str().unwrap_or_default();
            if d["settings"].get(a).is_some() {
                return Err(format!("{name}: {a} should not be in settings"));
            }
        }
    }
    if let Some(want) = expect["ignored"].as_array() {
        let have = d["ignored"].as_array().cloned().unwrap_or_default();
        for w in want {
            if !have.iter().any(|h| subset(w, h, "").is_ok()) {
                return Err(format!("{name}: ignored lacks {w}: {have:?}"));
            }
        }
    }
    Ok(())
}

#[test]
fn config_vectors() {
    let failures: Vec<String> = vectors("config")
        .iter()
        .filter_map(|(p, v)| run_config_vector(p, v).err())
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn config_path_vectors() {
    for (_, v) in vectors("config-path") {
        for c in v["checks"].as_array().unwrap() {
            let env = env_of(&c["env"], "");
            let code = c["code"]["config_file"].as_str();
            let got = config_path(os_of(&c["os"]), &env, c["home"].as_str(), code).map(|x| x.0);
            assert_eq!(got.as_deref(), c["expect"].as_str(), "{}", c["summary"]);
        }
    }
}

#[test]
fn duration_vectors() {
    for (_, v) in vectors("durations") {
        for c in v["checks"].as_array().unwrap() {
            let got = parse_duration(c["value"].as_str().unwrap());
            match c["expect"].as_u64() {
                Some(ms) => assert_eq!(got, Some(ms), "{}", c["value"]),
                None => assert_eq!(got, None, "{}", c["value"]),
            }
        }
    }
}

#[test]
fn the_error_lists_every_problem_and_never_a_secret() {
    let env: BTreeMap<String, String> = [
        ("INORBIT_TOKEN", "t"),
        ("INORBIT_TIMEOUT", "30"),
        ("INORBIT_CLIENT_KEY_PASSWORD", "pw-never-shown"),
        ("INORBIT_CONFIG_FILE", "off"),
        ("INORBIT_LOG", "loud"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let read = |_: &str| None;
    let e = resolve(&Inputs {
        env: &env,
        os: Os::Linux,
        home: None,
        cwd: "/".into(),
        code: Map::new(),
        profile_type: None,
        cli_found: &|_| false,
        read: &read,
    })
    .unwrap_err();
    let text = e.to_string();
    assert!(
        text.starts_with("configuration is invalid (2 problems):"),
        "{text}"
    );
    assert!(text.contains("timeout: \"30\" is not a duration; write it with a unit, such as 30s (from env INORBIT_TIMEOUT)"), "{text}");
    assert!(text.contains("log: \"loud\""), "{text}");
    assert!(!text.contains("pw-never-shown"));
}
