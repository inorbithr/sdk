//! `edge.caddy-site-call` (platform PRD 0204, ADR 0227): adds the voice call site
//! (`call.inorbit.hr`, proxied to the platform's `api` snippet) to the edge Caddy config on
//! the runner's host, validates the config and reloads Caddy.
//!
//! The site, the file and the container are fixed here, never params:
//!
//! 1. `docker inspect` must say [`CONTAINER`] is running and name the host folder it mounts
//!    at `/etc/caddy` (the edge checkout's `devops/edge`).
//! 2. That folder's `Caddyfile` must define the `(api)` snippet the block imports, and
//!    [`SITE_FILE`] must be a regular file there.
//! 3. When the file has no `call.inorbit.hr {` line, it is first copied to
//!    `inorbit.hr.caddy.bak-<UTC time>`, then [`BLOCK`] is appended in one rename.
//! 4. `caddy validate` in the container; a failure puts the backup back and stops.
//! 5. `caddy reload` in the container; a failure puts the backup back too (Caddy keeps
//!    the config it ran).
//! 6. `wget` in the container reads `https://call.inorbit.hr/rtc/validate` (through the
//!    public edge, as a phone reaches it) until it answers 401, at most [`TRIES`] times
//!    [`ArgvRunner::settle`] apart (about 60 s). A site that never answers 401 fails the
//!    run and stays in place: the config validated and Caddy runs it.

use std::path::{Path, PathBuf};

use time::OffsetDateTime;

use super::install_checks::{backup_path, replace};
use super::{Action, ArgvRunner, Tool, said, strings};

/// The edge Caddy container on the deploy server.
pub(crate) const CONTAINER: &str = "tbd-edge-caddy-1";

/// The site file, under the folder the container mounts at `/etc/caddy`.
pub(crate) const SITE_FILE: &str = "sites.d/inorbit.hr.caddy";

/// The site's first line, as the file holds it once added.
pub(crate) const SITE_LINE: &str = "call.inorbit.hr {";

/// What is appended: the call host behind the platform's `api` snippet.
pub(crate) const BLOCK: &str = "call.inorbit.hr {\n\timport api\n}\n";

/// The snippet the block imports; the Caddyfile must define it.
const SNIPPET_LINE: &str = "(api) {";

/// The config the container runs, as the container sees it.
const CONFIG: &str = "/etc/caddy/Caddyfile";

/// The Go template `docker inspect` answers with: running, a tab, the `/etc/caddy` source.
const INSPECT_FORMAT: &str = "{{.State.Running}}\t{{range .Mounts}}{{if eq .Destination \"/etc/caddy\"}}{{.Source}}{{end}}{{end}}";

/// The call host's token check: 401 without a token means the site reaches the platform.
const PROBE: &str = "https://call.inorbit.hr/rtc/validate";

/// How many times the probe runs before the run fails.
pub(crate) const TRIES: usize = 12;

/// The effect sentence; the action has no params.
pub(crate) const EFFECT: &str = "Adds the call site to the edge Caddy config on the runner's host, validates it and reloads Caddy.";

/// What runs on this system.
///
/// # Errors
/// Any system but Linux.
pub(crate) fn check(os: &str) -> Result<(), String> {
    if os == "linux" {
        Ok(())
    } else {
        Err("edge.caddy-site-call: Linux only".to_owned())
    }
}

/// Whether `text` has `line` as one of its lines, spaces around it ignored.
fn has_line(text: &str, line: &str) -> bool {
    text.lines().any(|l| l.trim() == line)
}

/// `current` with [`BLOCK`] after it, one blank line between.
pub(crate) fn appended(current: &str) -> String {
    let mut next = current.to_owned();
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    if !next.is_empty() {
        next.push('\n');
    }
    next.push_str(BLOCK);
    next
}

/// The host folder the container mounts at `/etc/caddy`, from what `docker inspect` said.
///
/// # Errors
/// The container is not running, or mounts no absolute plain folder there.
pub(crate) fn edge_dir(inspect: &str) -> Result<PathBuf, String> {
    let (running, source) = inspect
        .trim_end()
        .split_once('\t')
        .unwrap_or((inspect.trim(), ""));
    if running.trim() != "true" {
        return Err(format!("refused: {CONTAINER} is not running"));
    }
    // The mount is a path on the Linux host, so it is checked as one on every platform:
    // `Path::is_absolute` wants a drive letter on Windows.
    let dir = Path::new(source);
    if !source.starts_with('/') || dir.components().any(|c| c.as_os_str() == "..") {
        return Err(format!(
            "refused: {CONTAINER} mounts no host folder at /etc/caddy"
        ));
    }
    Ok(dir.to_path_buf())
}

/// One `docker exec <container> caddy ...` call, or why it failed.
async fn caddy(tools: &impl ArgvRunner, verb: &str) -> Result<(), String> {
    let argv = strings([
        "exec",
        CONTAINER,
        "caddy",
        verb,
        "--config",
        CONFIG,
        "--adapter",
        "caddyfile",
    ]);
    let out = tools.run(Tool::Docker, &argv).await?;
    if out.success {
        Ok(())
    } else {
        Err(format!("caddy {verb}: failed\n{}", said(&out))
            .trim_end()
            .to_owned())
    }
}

/// Whether what `wget -S` printed holds a 401 status line.
pub(crate) fn answered_401(stderr: &str) -> bool {
    stderr.lines().any(|l| {
        let l = l.trim();
        l.contains("HTTP/") && l.split_whitespace().any(|w| w == "401")
    })
}

/// The probe until it answers 401, or the last thing it said.
async fn probe(tools: &impl ArgvRunner) -> Result<(), String> {
    let argv = strings([
        "exec",
        CONTAINER,
        "wget",
        "-S",
        "-q",
        "-O",
        "/dev/null",
        "-T",
        "5",
        PROBE,
    ]);
    let mut last = String::new();
    for n in 0..TRIES {
        if n > 0 {
            tools.settle().await;
        }
        let out = tools.run(Tool::Docker, &argv).await?;
        if answered_401(&out.stderr) {
            return Ok(());
        }
        last = said(&out);
    }
    Err(
        format!("{PROBE} did not answer 401 in {TRIES} tries\n{last}")
            .trim_end()
            .to_owned(),
    )
}

/// Carries out `edge.caddy-site-call`.
///
/// # Errors
/// A refusal before the file is touched (container, snippet, file), or a failed validate
/// or reload, after the backup is put back.
pub(crate) async fn add_site(action: &Action, tools: &impl ArgvRunner) -> Result<String, String> {
    if !matches!(action, Action::EdgeSiteCall) {
        return Err("not edge.caddy-site-call".to_owned());
    }
    let out = tools
        .run(
            Tool::Docker,
            &strings(["inspect", "--format", INSPECT_FORMAT, CONTAINER]),
        )
        .await?;
    if !out.success {
        return Err(format!("docker inspect: failed\n{}", said(&out))
            .trim_end()
            .to_owned());
    }
    let dir = edge_dir(&out.stdout)?;
    let caddyfile = std::fs::read_to_string(dir.join("Caddyfile"))
        .map_err(|e| format!("refused: could not read the Caddyfile: {}", e.kind()))?;
    if !has_line(&caddyfile, SNIPPET_LINE) {
        return Err("refused: the Caddyfile defines no (api) snippet".to_owned());
    }
    let file = dir.join(SITE_FILE);
    let is_file = std::fs::symlink_metadata(&file).is_ok_and(|m| m.file_type().is_file());
    if !is_file {
        return Err(format!("refused: {SITE_FILE} is not a regular file"));
    }
    let current = std::fs::read_to_string(&file)
        .map_err(|e| format!("could not read {SITE_FILE}: {}", e.kind()))?;
    let backup = if has_line(&current, SITE_LINE) {
        None
    } else {
        let backup = backup_path(&file, OffsetDateTime::now_utc())?;
        std::fs::copy(&file, &backup)
            .map_err(|e| format!("could not back up {SITE_FILE}: {}", e.kind()))?;
        let candidate =
            file.with_file_name(format!(".inorbit.hr.caddy.next-{}", std::process::id()));
        std::fs::write(&candidate, appended(&current))
            .map_err(|e| format!("could not write {SITE_FILE}: {}", e.kind()))?;
        let moved = replace(&candidate, &file);
        let _ = std::fs::remove_file(&candidate);
        moved?;
        Some(backup)
    };
    let backup_name = backup
        .as_deref()
        .and_then(Path::file_name)
        .and_then(|n| n.to_str())
        .unwrap_or("the backup")
        .to_owned();
    let outcome = async {
        caddy(tools, "validate").await?;
        caddy(tools, "reload").await
    }
    .await;
    if let Err(why) = outcome {
        return Err(match &backup {
            None => format!("{why}; {SITE_FILE} already held the site and is unchanged"),
            Some(b) => match replace(b, &file) {
                Ok(()) => format!("{why}; {SITE_FILE} is back from {backup_name}"),
                Err(e) => format!(
                    "{why}; and {SITE_FILE} could not be put back ({e}): {backup_name} has it"
                ),
            },
        });
    }
    let done = match backup {
        None => {
            format!("call.inorbit.hr: already in {SITE_FILE}; Caddy config validated and reloaded")
        }
        Some(_) => format!(
            "call.inorbit.hr: added to {SITE_FILE} (old file kept as {backup_name}); Caddy config validated and reloaded"
        ),
    };
    probe(tools)
        .await
        .map_err(|why| format!("{done}, but {why}"))?;
    Ok(format!("{done}; /rtc/validate answers 401"))
}

#[cfg(test)]
mod tests;
