//! `iohr ext lint`: an extension's source manifest checked, and its permissions in
//! plain words (PRD 0084). Each rule reports on its own, so one run lists every
//! problem: an unknown key, a value that looks like a secret, a data kind outside ADR
//! 0063's non-sensitive list, a target the kind cannot be installed on, a `tbd` name,
//! and every rule of the artifact's config.

use std::fmt::Write as _;

use serde::Serialize;

use super::manifest::{
    DATA_LEAVES, Manifest, REPOS_VAR, RunAs, SOURCE_FILE, SourceManifest, TARGET_KINDS, data_leaf,
    printable, privilege,
};

/// One problem found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// `unknown_key`, `secret_value`, `data_class`, `target_kind`, `panel`, `tbd_name`
    /// or `invalid`.
    pub rule: &'static str,
    /// What is wrong and what to do.
    pub message: String,
}

/// What `iohr ext lint` found, and the preview when the manifest could be read.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// The manifest's `id`, when it has one.
    pub id: String,
    /// Its version.
    pub version: String,
    /// No findings.
    pub ok: bool,
    /// The problems.
    pub findings: Vec<Finding>,
    /// The permissions in plain words and structured; `None` when the manifest could
    /// not be read.
    pub preview: Option<Preview>,
    /// The artifact's config derived from the manifest; `None` unless `ok`.
    pub config: Option<Manifest>,
}

/// What an extension may do, structured, with the sentences an installer shows.
#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    /// The sentences, in order; `iohr ext lint --preview` prints exactly these.
    pub lines: Vec<String>,
    /// API scopes over the token channel.
    pub scopes: Vec<String>,
    /// Linux capabilities and what each lets it do.
    pub privileges: Vec<Words>,
    /// Hosts it may reach.
    pub network: Vec<String>,
    /// Paths it may read.
    pub read: Vec<String>,
    /// Paths it may write.
    pub write: Vec<String>,
    /// Programs it may start.
    pub exec: Vec<String>,
    /// Secrets it is given, by reference.
    pub secrets: Vec<SecretWords>,
    /// What it may send up.
    pub data_leaves: Vec<Words>,
    /// Where it may be installed.
    pub targets: Vec<String>,
    /// What is enforced and what is declared only.
    pub enforcement: String,
}

/// A name and its plain words.
#[derive(Debug, Clone, Serialize)]
pub struct Words {
    /// The name.
    pub name: String,
    /// What it means.
    pub words: String,
}

/// A secret by reference.
#[derive(Debug, Clone, Serialize)]
pub struct SecretWords {
    /// Its name.
    pub name: String,
    /// What the target resolves.
    #[serde(rename = "ref")]
    pub reference: String,
    /// The variable the program receives it in.
    pub env: String,
}

/// Lints the text of `extension.toml`.
#[must_use]
pub fn lint(text: &str) -> Report {
    let mut findings = Vec::new();
    let raw: Option<toml::Table> = match toml::from_str::<toml::Table>(text) {
        Ok(t) => Some(t),
        Err(e) => {
            findings.push(Finding {
                rule: "invalid",
                message: format!("{SOURCE_FILE} is not TOML: {}", e.message().trim()),
            });
            None
        }
    };
    if let Some(raw) = &raw {
        scan_secrets(&toml::Value::Table(raw.clone()), "", &mut findings);
        raw_rules(raw, &mut findings);
    }
    let mut report = Report {
        id: raw
            .as_ref()
            .and_then(|r| r.get("id"))
            .and_then(toml::Value::as_str)
            .map(printable)
            .unwrap_or_default(),
        version: raw
            .as_ref()
            .and_then(|r| r.get("version"))
            .and_then(toml::Value::as_str)
            .map(printable)
            .unwrap_or_default(),
        ok: false,
        findings: Vec::new(),
        preview: None,
        config: None,
    };
    if raw.is_some() {
        match SourceManifest::parse(text) {
            Err(e) => findings.push(Finding {
                rule: if e.0.contains("unknown field") {
                    "unknown_key"
                } else {
                    "invalid"
                },
                message: e.0,
            }),
            Ok(source) => {
                report.preview = Some(preview(&source));
                // The rules reported above are left out here, so each problem is listed
                // once.
                let mut checked = source.clone();
                checked
                    .permissions
                    .data
                    .leaves
                    .retain(|l| data_leaf(l).is_some());
                if let Some(t) = &mut checked.targets {
                    t.kinds.retain(|k| supported(&source.kind, k));
                }
                match checked.to_manifest() {
                    Ok(m) => report.config = Some(m),
                    Err(e) => findings.push(Finding {
                        rule: "invalid",
                        message: e.0,
                    }),
                }
            }
        }
    }
    report.ok = findings.is_empty();
    if !report.ok {
        report.config = None;
    }
    report.findings = findings;
    report
}

fn supported(kind: &str, target: &str) -> bool {
    TARGET_KINDS
        .iter()
        .find(|(k, _)| *k == kind)
        .is_some_and(|(_, ts)| ts.contains(&target))
}

/// The rules for a `cluster` target and a `[panel]` (RFC 0143).
fn cluster_and_panel_rules(raw: &toml::Table, kind: &str, findings: &mut Vec<Finding>) {
    let kinds = raw
        .get("targets")
        .and_then(|t| t.get("kinds"))
        .and_then(toml::Value::as_array);
    let clustered = kinds
        .into_iter()
        .flatten()
        .any(|t| t.as_str() == Some("cluster"));
    if clustered && supported(kind, "cluster") {
        // A cluster target is a pod of the same signed artifact (RFC 0143, section 6).
        let delivery = raw
            .get("delivery")
            .and_then(toml::Value::as_str)
            .unwrap_or("program");
        if delivery != "program" {
            findings.push(Finding {
                rule: "target_kind",
                message: format!(
                    "targets.kinds: `cluster` runs the program artifact as a pod; delivery is `{}`, not `program`",
                    printable(delivery)
                ),
            });
        }
        let platforms = raw
            .get("targets")
            .and_then(|t| t.get("platforms"))
            .and_then(toml::Value::as_array);
        if let Some(p) = platforms
            && !p
                .iter()
                .any(|x| x.as_str().is_some_and(|x| x.starts_with("linux/")))
        {
            findings.push(Finding {
                rule: "target_kind",
                message: "targets.kinds: `cluster` runs on Linux: add a `linux/...` platform, such as linux/amd64, to targets.platforms".into(),
            });
        }
    }
    if raw.contains_key("panel") && kind != "program" {
        findings.push(Finding {
            rule: "panel",
            message: format!(
                "[panel]: only a program serves a panel, and this is a {}",
                printable(kind)
            ),
        });
    }
}

/// The rules read from the raw table, so they report even when the manifest's shape is
/// wrong elsewhere.
fn raw_rules(raw: &toml::Table, findings: &mut Vec<Finding>) {
    let id = raw.get("id").and_then(toml::Value::as_str).unwrap_or("");
    if contains_tbd(id) {
        findings.push(Finding {
            rule: "tbd_name",
            message: format!(
                "`{}` holds \"tbd\": name the extension for what it does before it is built",
                printable(id)
            ),
        });
    }
    let leaves = raw
        .get("permissions")
        .and_then(|p| p.get("data"))
        .and_then(|d| d.get("leaves"))
        .and_then(toml::Value::as_array);
    for l in leaves.into_iter().flatten() {
        let l = l.as_str().unwrap_or("");
        if data_leaf(l).is_none() {
            findings.push(Finding {
                rule: "data_class",
                message: format!(
                    "permissions.data.leaves: `{}` is not in ADR 0063's non-sensitive list ({}); \
                     anything else leaves only through a per-item share",
                    printable(l),
                    DATA_LEAVES
                        .iter()
                        .map(|(k, _)| *k)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }
    }
    let kind = raw
        .get("kind")
        .and_then(toml::Value::as_str)
        .unwrap_or("program");
    if !TARGET_KINDS.iter().any(|(k, _)| *k == kind) {
        findings.push(Finding {
            rule: "target_kind",
            message: format!(
                "kind `{}` is not one of {}",
                printable(kind),
                TARGET_KINDS
                    .iter()
                    .map(|(k, _)| *k)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
        return;
    }
    let kinds = raw
        .get("targets")
        .and_then(|t| t.get("kinds"))
        .and_then(toml::Value::as_array);
    cluster_and_panel_rules(raw, kind, findings);
    for t in kinds.into_iter().flatten() {
        let t = t.as_str().unwrap_or("");
        if !supported(kind, t) {
            let allowed = TARGET_KINDS
                .iter()
                .find(|(k, _)| *k == kind)
                .map(|(_, ts)| ts.join(", "))
                .unwrap_or_default();
            findings.push(Finding {
                rule: "target_kind",
                message: format!(
                    "targets.kinds: a {kind} cannot be installed on `{}`{}",
                    printable(t),
                    if allowed.is_empty() {
                        " (it is not installed on a target)".to_owned()
                    } else {
                        format!(" (only {allowed})")
                    }
                ),
            });
        }
    }
}

/// "tbd" anywhere, in any case.
#[must_use]
pub fn contains_tbd(s: &str) -> bool {
    s.to_ascii_lowercase().contains("tbd")
}

fn scan_secrets(v: &toml::Value, path: &str, findings: &mut Vec<Finding>) {
    match v {
        toml::Value::String(s) => {
            if let Some(why) = secret_shaped(s) {
                findings.push(Finding {
                    rule: "secret_value",
                    message: format!(
                        "{} looks like {why}: a manifest holds secret references (secrets = [{{ name, ref, env }}]), never values",
                        if path.is_empty() { "a value" } else { path }
                    ),
                });
            }
        }
        toml::Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                scan_secrets(x, &format!("{path}[{i}]"), findings);
            }
        }
        toml::Value::Table(t) => {
            for (k, x) in t {
                let p = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                scan_secrets(x, &p, findings);
            }
        }
        _ => {}
    }
}

/// Prefixes of well-known tokens and keys.
const PREFIXES: &[(&str, &str)] = &[
    ("ghp_", "a GitHub token"),
    ("gho_", "a GitHub token"),
    ("ghu_", "a GitHub token"),
    ("ghs_", "a GitHub token"),
    ("ghr_", "a GitHub token"),
    ("github_pat_", "a GitHub token"),
    ("glpat-", "a GitLab token"),
    ("xoxb-", "a Slack token"),
    ("xoxp-", "a Slack token"),
    ("xapp-", "a Slack token"),
    ("sk-", "an API secret key"),
    ("sk_live_", "an API secret key"),
    ("sk_test_", "an API secret key"),
    ("rk_live_", "an API secret key"),
    ("npm_", "an npm token"),
    ("pypi-", "a PyPI token"),
    ("aiza", "a Google API key"),
    ("ya29.", "a Google access token"),
    ("akia", "an AWS access key"),
    ("asia", "an AWS access key"),
];

/// Why a value looks like a secret: a token, a key, a URL with user info or a
/// connection string with a password; `None` when it does not.
#[must_use]
pub fn secret_shaped(s: &str) -> Option<&'static str> {
    let t = s.trim();
    let lower = t.to_ascii_lowercase();
    if t.contains("-----BEGIN") {
        return Some("a PEM key or certificate");
    }
    for (p, why) in PREFIXES {
        if lower.starts_with(p) && t.len() >= p.len() + 12 && !t.contains(char::is_whitespace) {
            return Some(why);
        }
    }
    if t.starts_with("eyJ") && t.matches('.').count() == 2 && t.len() > 30 {
        return Some("a JSON web token");
    }
    if let Some((scheme, rest)) = t.split_once("://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        if !scheme.is_empty()
            && scheme
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'+')
            && authority.contains('@')
        {
            return Some("a URL with a user name or password in it");
        }
    }
    for k in [
        "password=",
        "pwd=",
        "passwd=",
        "accountkey=",
        "sharedaccesskey=",
        "secret=",
    ] {
        if lower.contains(k) {
            return Some("a connection string with a password");
        }
    }
    let charset = t
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'_' | b'-'));
    if t.len() >= 40 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some("a long hex key");
    }
    if t.len() >= 32
        && charset
        && t.bytes().any(|b| b.is_ascii_uppercase())
        && t.bytes().any(|b| b.is_ascii_lowercase())
        && t.bytes().any(|b| b.is_ascii_digit())
    {
        return Some("a random key or token");
    }
    None
}

/// "a, b and c".
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// `$STATE` and `$CACHE` only, as its own.
fn own(paths: &[String]) -> bool {
    paths.iter().all(|p| p == "$STATE" || p == "$CACHE")
}

/// The preview of a manifest that could be read.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one sentence per permission, in the order shown"
)]
pub fn preview(s: &SourceManifest) -> Preview {
    let p = &s.permissions;
    let mut lines = Vec::new();
    lines.push(format!(
        "{} {}{}",
        s.id,
        s.version,
        if s.description.is_empty() {
            String::new()
        } else {
            format!(": {}", s.description)
        }
    ));
    lines.push(if p.scopes.is_empty() {
        "Asks iohr for no API access.".to_owned()
    } else {
        format!(
            "May ask iohr for API tokens with {}, and nothing else.",
            and_list(&p.scopes)
        )
    });
    let privileges: Vec<Words> = p
        .privileges
        .iter()
        .map(|n| Words {
            name: n.clone(),
            words: privilege(n).unwrap_or("unknown").to_owned(),
        })
        .collect();
    lines.push(if privileges.is_empty() {
        "Holds no special privileges.".to_owned()
    } else {
        format!(
            "Its system service holds privileges: {}.",
            privileges
                .iter()
                .map(|w| format!("{} ({})", w.name, w.words))
                .collect::<Vec<_>>()
                .join("; ")
        )
    });
    lines.push(if p.network.is_empty() {
        "Reaches no network host.".to_owned()
    } else {
        format!("May reach {}.", and_list(&p.network))
    });
    let (read, write) = (&p.filesystem.read, &p.filesystem.write);
    let repos = read.iter().chain(write).any(|x| x == REPOS_VAR);
    lines.push(if read.is_empty() && write.is_empty() {
        "Reads and writes no files.".to_owned()
    } else if own(read) && own(write) {
        if write.is_empty() {
            "May read its own state, nowhere else.".to_owned()
        } else {
            "May read and write its own state and cache, nowhere else.".to_owned()
        }
    } else {
        let mut l = String::new();
        if !read.is_empty() {
            let _ = write!(l, "May read {}.", and_list(read));
        }
        if !write.is_empty() {
            if !l.is_empty() {
                l.push(' ');
            }
            let _ = write!(l, "May write {}.", and_list(write));
        }
        l
    });
    if repos {
        lines.push(
            "$REPOS is the repository folders you add to it in its host; none until you add one."
                .to_owned(),
        );
    }
    lines.push(if p.exec.is_empty() {
        "Starts no other program.".to_owned()
    } else {
        format!("May start {}.", and_list(&p.exec))
    });
    let secrets: Vec<SecretWords> = s
        .secrets
        .iter()
        .map(|x| SecretWords {
            name: x.name.clone(),
            reference: x.reference.clone(),
            env: x.env.clone(),
        })
        .collect();
    lines.push(match secrets.as_slice() {
        [] => "Asks for no secrets.".to_owned(),
        [one] => format!(
            "Asks for one secret, {}, given to it as {}.",
            one.reference, one.env
        ),
        many => format!(
            "Asks for {} secrets: {}.",
            many.len(),
            and_list(
                &many
                    .iter()
                    .map(|x| format!("{} as {}", x.reference, x.env))
                    .collect::<Vec<_>>()
            )
        ),
    });
    let data_leaves: Vec<Words> = p
        .data
        .leaves
        .iter()
        .map(|l| Words {
            name: l.clone(),
            words: data_leaf(l)
                .unwrap_or("not allowed: outside ADR 0063's non-sensitive list")
                .to_owned(),
        })
        .collect();
    lines.push(if data_leaves.is_empty() {
        "Sends nothing up.".to_owned()
    } else {
        format!(
            "Sends up: {}.",
            data_leaves
                .iter()
                .map(|w| w.words.clone())
                .collect::<Vec<_>>()
                .join("; ")
        )
    });
    if let Some(r) = &s.runs {
        lines.push(match r.run_as {
            RunAs::Service => format!(
                "Runs as a service, restarted {}.",
                match r.restart {
                    super::manifest::Restart::Always => "whenever it stops",
                    super::manifest::Restart::OnFailure => "when it fails",
                    super::manifest::Restart::Never => "never",
                }
            ),
            RunAs::Timer => format!("Runs every {}.", r.every),
            RunAs::Oneshot => "Runs once.".to_owned(),
        });
    }
    if let Some(panel) = &s.panel {
        lines.push(format!(
            "Serves a panel, \"{}\", on a loopback address for its host to show.",
            printable(&panel.title)
        ));
    }
    let targets: Vec<String> = s
        .targets
        .as_ref()
        .map(|t| t.kinds.clone())
        .unwrap_or_default();
    if let Some(t) = &s.targets {
        let words: Vec<String> = t
            .kinds
            .iter()
            .map(|k| match k.as_str() {
                "desktop" => "this computer".to_owned(),
                "host" => "a host over SSH".to_owned(),
                "agent" => "a customer agent".to_owned(),
                "cluster" => "a pod in the cluster".to_owned(),
                other => other.to_owned(),
            })
            .collect();
        if !words.is_empty() {
            lines.push(format!("May be installed on {}.", and_list(&words)));
        }
        if !t.needs.is_empty() {
            lines.push(format!("Needs {} on the target.", and_list(&t.needs)));
        }
    }
    let enforcement = "Scopes are enforced by iohr's token channel. Network hosts, files and \
                       programs are declared and shown, not yet enforced; on macOS they stay \
                       declared and audited only."
        .to_owned();
    lines.push(enforcement.clone());
    Preview {
        lines,
        scopes: p.scopes.clone(),
        privileges,
        network: p.network.clone(),
        read: read.clone(),
        write: write.clone(),
        exec: p.exec.clone(),
        secrets,
        data_leaves,
        targets,
        enforcement,
    }
}

#[cfg(test)]
mod tests {
    use super::{lint, secret_shaped};

    const BASE: &str = r#"
id = "inorbit/hello"
version = "0.1.0"
entrypoint = "hello"
description = "Says hello."

[permissions]
scopes = ["identity:read"]
network = ["api.github.com:443"]
exec = ["git"]

[permissions.filesystem]
read = ["$STATE"]
write = ["$STATE", "$CACHE"]

[permissions.data]
leaves = ["check.result"]

[targets]
kinds = ["desktop", "host"]

[dev]
lang = "shell"
"#;

    fn rules(text: &str) -> Vec<&'static str> {
        lint(text).findings.iter().map(|f| f.rule).collect()
    }

    #[test]
    fn a_good_manifest_passes_with_a_preview_and_a_config() {
        let r = lint(BASE);
        assert!(r.ok, "{:?}", r.findings);
        let p = r.preview.unwrap();
        assert!(
            p.lines.iter().any(|l| l == "May reach api.github.com:443."),
            "{:?}",
            p.lines
        );
        assert!(p.lines.iter().any(|l| l == "May start git."));
        assert!(
            p.lines
                .iter()
                .any(|l| l == "Sends up: check results (status, timings, ids).")
        );
        let c = serde_json::to_value(r.config.unwrap()).unwrap();
        assert_eq!(c["name"], "hello");
        assert_eq!(c["scopes"], serde_json::json!(["identity:read"]));
        assert!(c["permissions"].get("scopes").is_none(), "{c}");
    }

    #[test]
    fn each_rule_fails_on_its_own() {
        let cases = [
            (
                BASE.replace("inorbit/hello", "inorbit/hello-tbd"),
                "tbd_name",
            ),
            (BASE.replace("check.result", "request.body"), "data_class"),
            (
                BASE.replace(r#""desktop", "host""#, r#""desktop", "phone""#),
                "target_kind",
            ),
            (
                BASE.replace("description =", "colour = \"red\"\ndescription ="),
                "unknown_key",
            ),
            (
                BASE.replace("exec = [\"git\"]", "exec = [\"git\"]\nshell = true"),
                "unknown_key",
            ),
            (
                BASE.replace("Says hello.", "ghp_abcdefghijklmnopqrstuvwxyz0123456789"),
                "secret_value",
            ),
            (
                BASE.replace("api.github.com:443", "postgres://app:hunter2@db:5432"),
                "secret_value",
            ),
            (BASE.replace("0.1.0", "one"), "invalid"),
        ];
        for (text, rule) in cases {
            let found = rules(&text);
            assert!(found.contains(&rule), "{rule}: {found:?}");
            assert!(!lint(&text).ok);
            assert!(lint(&text).config.is_none());
        }
    }

    #[test]
    fn secret_shapes() {
        for s in [
            "ghp_0123456789abcdefghijklmnopqrstuvwxyz",
            "github_pat_11ABCDEFG0123456789_abcdefghijklmnop",
            "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJhIn0.c2lnbmF0dXJl",
            "https://user:pass@example.com/x",
            "Server=db;User Id=sa;Password=hunter2;",
            "AKIAABCDEFGHIJKLMNOP",
            "-----BEGIN PRIVATE KEY-----",
            "q8Zr2LmN4vX7tY1wK9pH3sD6fG0jB5cA",
            "0123456789abcdef0123456789abcdef01234567",
        ] {
            assert!(secret_shaped(s).is_some(), "{s}");
        }
        for s in [
            "identity:read",
            "$STATE/heartbeat.json",
            "api.github.com:443",
            "https://github.com/inorbithr/sdk",
            "Writes a heartbeat every minute.",
            "darwin/arm64",
            "github-token",
            "GITHUB_TOKEN",
            ">=2026.10 <2027.01",
            "weekdays 17:00",
        ] {
            assert!(secret_shaped(s).is_none(), "{s}");
        }
    }

    fn with_panel(extra: &str) -> String {
        format!("{BASE}\n[panel]\n{extra}\n")
    }

    #[test]
    fn a_cluster_program_with_a_panel_and_repos_passes() {
        let text = BASE
            .replace(r#""desktop", "host""#, r#""desktop", "cluster""#)
            .replace(r#"read = ["$STATE"]"#, r#"read = ["$STATE", "$REPOS"]"#)
            .replace(
                "[targets]",
                "[targets]\nplatforms = [\"darwin/arm64\", \"linux/amd64\"]",
            )
            + "\n[panel]\ntitle = \"Todo\"\nentry = \"/\"\n";
        let r = lint(&text);
        assert!(r.ok, "{:?}", r.findings);
        let lines = r.preview.unwrap().lines;
        assert!(
            lines
                .iter()
                .any(|l| l.contains("$REPOS is the repository folders")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains(r#"Serves a panel, "Todo""#)),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("a pod in the cluster")),
            "{lines:?}"
        );
    }

    #[test]
    fn cluster_is_for_a_program_only() {
        let program = BASE.replace(r#""desktop", "host""#, r#""desktop", "cluster""#);
        assert!(lint(&program).ok, "{:?}", lint(&program).findings);
        for kind in ["agent-plugin", "web"] {
            let text = program
                .replace("id = ", &format!("kind = \"{kind}\"\nid = "))
                .replace("\n[dev]\nlang = \"shell\"\n", "");
            let r = lint(&text);
            assert!(
                r.findings
                    .iter()
                    .any(|f| f.rule == "target_kind" && f.message.contains("on `cluster`")),
                "{kind}: {:?}",
                r.findings
            );
        }
    }

    #[test]
    fn a_cluster_target_needs_program_delivery_and_a_linux_platform() {
        let cluster = BASE.replace(r#""desktop", "host""#, r#""desktop", "cluster""#);
        let no_linux = cluster.replace("[targets]", "[targets]\nplatforms = [\"darwin/arm64\"]");
        let r = lint(&no_linux);
        assert!(
            r.findings
                .iter()
                .any(|f| f.message.contains("add a `linux/...` platform")),
            "{:?}",
            r.findings
        );
        let r = lint(&cluster.replace("id = ", "delivery = \"binary\"\nid = "));
        assert!(
            r.findings
                .iter()
                .any(|f| f.message.contains("delivery is `binary`")),
            "{:?}",
            r.findings
        );
    }

    #[test]
    fn a_panel_is_checked() {
        assert!(lint(&with_panel("title = \"Todo\"\nentry = \"/\"")).ok);
        // an unknown key, and a missing entry
        assert!(
            rules(&with_panel("title = \"Todo\"\nentry = \"/\"\nicon = \"x\""))
                .contains(&"unknown_key")
        );
        let r = lint(&with_panel("title = \"Todo\""));
        assert!(!r.ok);
        assert!(
            r.findings.iter().any(|f| f.message.contains("entry")),
            "{:?}",
            r.findings
        );
        for bad in [
            "entry = \"todo\"",
            "entry = \"//evil\"",
            "entry = \"/a/../b\"",
            "entry = \"/a b\"",
        ] {
            let r = lint(&with_panel(&format!("title = \"Todo\"\n{bad}")));
            assert!(!r.ok, "{bad}");
        }
        assert!(!lint(&with_panel("title = \"\"\nentry = \"/\"")).ok);
    }

    #[test]
    fn a_panel_on_something_that_is_not_a_program_is_refused() {
        let text = BASE
            .replace("id = ", "kind = \"web\"\nid = ")
            .replace("\n[dev]\nlang = \"shell\"\n", "")
            .replace(r#"kinds = ["desktop", "host"]"#, "kinds = []")
            + "\n[panel]\ntitle = \"Todo\"\nentry = \"/\"\n";
        assert!(
            rules(&text).contains(&"panel"),
            "{:?}",
            lint(&text).findings
        );
    }
}
