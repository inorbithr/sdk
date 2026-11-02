//! `disk.prune` (platform RFC 0172): frees disk on the runner's host from one named
//! target and nothing else. It replaces the hand-written `~/.inorbit/asks/free-disk*.sh`
//! scripts for what core's own tasks already do; it deletes no path of its own, so
//! nothing the catalog does not name (and never `/mnt/fast`) can go.
//!
//! | target | what runs |
//! |---|---|
//! | `host` | `mise run host:prune` in the `core_checkout`: Docker's build cache and unused images, the k3d registry's old tags, unused k3d node images, stale shared build files |
//! | `docker-builder` | `docker builder prune -af --filter until=<until_hours>h` |
//! | `k3d-images` | `docker exec <node> crictl rmi --prune` in each running k3d node |
//!
//! Steps: the Docker data root is read (`docker info`; the root disk when Docker does not
//! say), its free bytes are read with `df` (the precondition), the target runs, the free
//! bytes are read again (the post check) and both numbers are reported. A run that freed
//! nothing fails, so an ask never reads as done when the disk is as full as it was.

use super::{
    Action, ArgvRunner, DISK_PRUNE, MAX_UNTIL_HOURS, Tool, closed, integer_param, redact_output,
    said, string_param, strings,
};

/// The closed set of targets.
pub(crate) const TARGETS: [&str; 3] = ["host", "docker-builder", "k3d-images"];

/// Whether `s` is a k3d node container name: `^k3d-[a-z0-9-]+-(server|agent)-[0-9]+$`, the
/// pattern core's `target-sweep.sh` uses (`TARGET_K3D_NODES_RE`).
pub(crate) fn is_k3d_node(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("k3d-") else {
        return false;
    };
    let Some((name_and_role, index)) = rest.rsplit_once('-') else {
        return false;
    };
    let role_ok = name_and_role
        .strip_suffix("-server")
        .or_else(|| name_and_role.strip_suffix("-agent"))
        .is_some_and(|name| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        });
    role_ok && !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit())
}

/// The params and the system, checked again before anything runs.
///
/// # Errors
/// An unknown target, `until_hours` on a target that has none (or missing on
/// `docker-builder`), or a system that is not Linux.
pub(crate) fn check(target: &str, until_hours: Option<u64>, os: &str) -> Result<(), String> {
    if os != "linux" {
        return Err(format!(
            "{DISK_PRUNE}: only a Linux runner prunes its host's disk this way; this one runs on {os}"
        ));
    }
    if !TARGETS.contains(&target) {
        return Err("target: not a target of the catalog".to_owned());
    }
    match (target, until_hours) {
        ("docker-builder", Some(h)) if (1..=MAX_UNTIL_HOURS).contains(&h) => Ok(()),
        ("docker-builder", _) => Err(format!("until_hours: needed, from 1 to {MAX_UNTIL_HOURS}")),
        (_, Some(_)) => Err("until_hours: only with docker-builder".to_owned()),
        (_, None) => Ok(()),
    }
}

/// The params of a signed message, checked against the schema's rules.
///
/// # Errors
/// An unknown param, a target outside the catalog, or `until_hours` out of place.
pub(crate) fn params(params: &serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    closed(params, &["target", "until_hours"], "/params")?;
    let target = string_param(
        params,
        "target",
        |s| TARGETS.contains(&s),
        "a target of the catalog",
    )?;
    if target == "docker-builder" {
        integer_param(params, "until_hours", 1, MAX_UNTIL_HOURS)?;
    } else if params.contains_key("until_hours") {
        return Err("/params/until_hours: only with docker-builder".to_owned());
    }
    Ok(())
}

/// A path `docker info` printed that may be handed to `df`: absolute, one line, no control
/// character.
fn is_data_root(s: &str) -> bool {
    s.starts_with('/') && s.len() <= 4096 && !s.chars().any(char::is_control)
}

/// The disk to measure: Docker's data root, or `/` when Docker does not say.
async fn data_root(tools: &impl ArgvRunner) -> String {
    let argv = strings(["info", "--format", "{{.DockerRootDir}}"]);
    match tools.run(Tool::Docker, &argv).await {
        Ok(out) if out.success && is_data_root(out.stdout.trim()) => out.stdout.trim().to_owned(),
        _ => "/".to_owned(),
    }
}

/// The free bytes on the disk that holds `root`.
async fn free_bytes(tools: &impl ArgvRunner, root: &str) -> Result<u64, String> {
    let argv = strings(["--output=avail", "-B1", "--", root]);
    let out = tools.run(Tool::Df, &argv).await?;
    if !out.success {
        return Err(format!("df: failed\n{}", said(&out)).trim_end().to_owned());
    }
    out.stdout
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .and_then(|l| l.parse::<u64>().ok())
        .ok_or_else(|| "df: did not say the free bytes".to_owned())
}

/// The running k3d nodes' names.
async fn nodes(tools: &impl ArgvRunner) -> Result<Vec<String>, String> {
    let out = tools
        .run(Tool::Docker, &strings(["ps", "--format", "{{.Names}}"]))
        .await?;
    if !out.success {
        return Err(format!("docker ps: failed\n{}", said(&out))
            .trim_end()
            .to_owned());
    }
    let found: Vec<String> = out
        .stdout
        .lines()
        .map(str::trim)
        .filter(|n| is_k3d_node(n))
        .map(str::to_owned)
        .collect();
    if found.is_empty() {
        return Err("no k3d node is running on this host".to_owned());
    }
    Ok(found)
}

/// One prune call: its summary line, or the failure with what the tool said.
async fn call(tools: &impl ArgvRunner, tool: Tool, argv: &[String]) -> Result<String, String> {
    let out = tools.run(tool, argv).await?;
    let what = said(&out);
    if !out.success {
        let head: Vec<&str> = argv.iter().take(2).map(String::as_str).collect();
        return Err(
            format!("{} {}: failed\n{what}", tool.name(), head.join(" "))
                .trim_end()
                .to_owned(),
        );
    }
    Ok(out
        .tally
        .total
        .as_deref()
        .map(redact_output)
        .unwrap_or_default())
}

/// Carries out `disk.prune`.
///
/// # Errors
/// A refusal, a failed call, or a run that freed nothing.
pub(crate) async fn prune(action: &Action, tools: &impl ArgvRunner) -> Result<String, String> {
    let Action::DiskPrune {
        target,
        until_hours,
    } = action
    else {
        return Err("not disk.prune".to_owned());
    };
    check(target, *until_hours, "linux")?;
    let root = data_root(tools).await;
    let before = free_bytes(tools, &root).await?;
    let mut said_by_tools = Vec::new();
    match target.as_str() {
        "host" => {
            said_by_tools.push(call(tools, Tool::CoreMise, &strings(["run", "host:prune"])).await?);
        }
        "docker-builder" => {
            let hours = until_hours.unwrap_or_default();
            let argv = strings([
                "builder",
                "prune",
                "-af",
                "--filter",
                &format!("until={hours}h"),
            ]);
            said_by_tools.push(call(tools, Tool::Docker, &argv).await?);
        }
        _ => {
            for node in nodes(tools).await? {
                let argv = strings(["exec", &node, "crictl", "rmi", "--prune"]);
                call(tools, Tool::Docker, &argv).await?;
            }
        }
    }
    let after = free_bytes(tools, &root).await?;
    if after <= before {
        return Err(format!(
            "{target}: nothing was freed ({before} bytes free before, {after} after)"
        ));
    }
    let mut line = format!(
        "{target}: {before} bytes free before, {after} after, {} freed",
        after - before
    );
    for extra in said_by_tools.into_iter().filter(|s| !s.is_empty()) {
        line.push('\n');
        line.push_str(&extra);
    }
    Ok(line)
}

#[cfg(test)]
mod tests;
