//! `helm.runner-scale` (platform RFC 0172): sets the most and the fewest runners of one
//! Actions Runner Controller scale set on the cluster the runner's config names. It
//! replaces the supervisor's by-hand `helm upgrade <release> ... --reuse-values --set
//! maxRunners=N --set minRunners=0 --force-conflicts`.
//!
//! The release is one of [`RELEASES`], in the `arc-runners` namespace; the chart is the
//! one core installs it from (`oci://ghcr.io/actions/actions-runner-controller-charts/
//! gha-runner-scale-set`, core's `runners:install`), at the version the release already
//! runs, read with `helm get metadata` (so a scale never changes the chart). Steps:
//!
//! 1. `helm get metadata` must say the release is the `gha-runner-scale-set` chart in
//!    `arc-runners`, and at what version.
//! 2. `helm get values` for what it holds now (reported, never required).
//! 3. `helm upgrade ... --reuse-values --set maxRunners=<n> --set minRunners=<m>
//!    --force-conflicts`: every other value of the release stays.
//! 4. `helm get values` again; the run fails unless both numbers read back as set.
//!
//! `helm` is the host's own (a `mise` shim, started in the `core_checkout` so it resolves
//! the version core pins) and every call is pinned to the configured cluster.

use serde_json::Value;

use super::{
    Action, ArgvRunner, RUNNER_SCALE, Tool, closed, integer_param, said, string_param, strings,
};

/// The runner scale sets this action may change: the releases core installs on the deploy
/// server.
pub(crate) const RELEASES: [&str; 2] = ["core-ci", "core-images"];

/// The largest `maxRunners` and `minRunners`.
pub(crate) const MAX_RUNNERS: u64 = 32;

/// The namespace of the runner scale sets.
const NAMESPACE: &str = "arc-runners";

/// The chart's name, as `helm get metadata` says it.
const CHART: &str = "gha-runner-scale-set";

/// Where the chart comes from (core's `runners:install`).
const CHART_REF: &str =
    "oci://ghcr.io/actions/actions-runner-controller-charts/gha-runner-scale-set";

/// The params, checked again before anything runs.
///
/// # Errors
/// A release outside the catalog, a count outside 1 to 32 (0 to 32 for the fewest), or a
/// fewest above the most.
pub(crate) fn check(release: &str, max: u64, min: u64) -> Result<(), String> {
    if !RELEASES.contains(&release) {
        return Err("release: not a runner scale set of the catalog".to_owned());
    }
    if !(1..=MAX_RUNNERS).contains(&max) {
        return Err(format!("max_runners: not from 1 to {MAX_RUNNERS}"));
    }
    if min > max {
        return Err("min_runners: above max_runners".to_owned());
    }
    Ok(())
}

/// The params of a signed message, checked against the schema's rules.
///
/// # Errors
/// An unknown param, a release outside the catalog, a count out of range, or a fewest
/// above the most.
pub(crate) fn params(params: &serde_json::Map<String, Value>) -> Result<(), String> {
    closed(
        params,
        &["release", "max_runners", "min_runners"],
        "/params",
    )?;
    string_param(
        params,
        "release",
        |s| RELEASES.contains(&s),
        "a runner scale set",
    )?;
    let max = integer_param(params, "max_runners", 1, MAX_RUNNERS)?;
    if params.contains_key("min_runners")
        && integer_param(params, "min_runners", 0, MAX_RUNNERS)? > max
    {
        return Err("/params/min_runners: above max_runners".to_owned());
    }
    Ok(())
}

/// `^[0-9]+\.[0-9]+\.[0-9]+$`: a chart version, so it is never read as a flag.
fn is_chart_version(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 6 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// What one `helm` call printed as JSON, or why it failed.
async fn helm_json(tools: &impl ArgvRunner, argv: &[String]) -> Result<Value, String> {
    let out = tools.run(Tool::Helm, argv).await?;
    if !out.success {
        return Err(format!("helm {}: failed\n{}", argv[0], said(&out))
            .trim_end()
            .to_owned());
    }
    serde_json::from_str(out.stdout.trim())
        .map_err(|_| format!("helm {}: did not answer JSON", argv[0]))
}

/// The count a release's values hold under `key`.
fn held(values: &Value, key: &str) -> Option<u64> {
    values.get(key)?.as_u64()
}

/// What the release holds, for a line a person reads.
fn show(values: &Value) -> String {
    let n = |k: &str| held(values, k).map_or_else(|| "unset".to_owned(), |n| n.to_string());
    format!(
        "maxRunners {}, minRunners {}",
        n("maxRunners"),
        n("minRunners")
    )
}

/// Carries out `helm.runner-scale`.
///
/// # Errors
/// A refusal, a failed call, a release that is not the scale-set chart, or values that do
/// not read back as set.
pub(crate) async fn scale(action: &Action, tools: &impl ArgvRunner) -> Result<String, String> {
    let Action::RunnerScale {
        release,
        max_runners,
        min_runners,
    } = action
    else {
        return Err("not helm.runner-scale".to_owned());
    };
    check(release, *max_runners, *min_runners)?;
    let meta = helm_json(
        tools,
        &strings(["get", "metadata", release, "-n", NAMESPACE, "-o", "json"]),
    )
    .await?;
    let chart = meta
        .get("chart")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let version = meta
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if chart != CHART {
        return Err(format!("refused: {release} is not a {CHART} release"));
    }
    if meta
        .get("namespace")
        .and_then(Value::as_str)
        .is_some_and(|ns| ns != NAMESPACE)
    {
        return Err(format!("refused: {release} is not in {NAMESPACE}"));
    }
    if !is_chart_version(version) {
        return Err(format!(
            "refused: helm did not say {release}'s chart version"
        ));
    }
    let values_argv = strings(["get", "values", release, "-n", NAMESPACE, "-o", "json"]);
    let was = helm_json(tools, &values_argv)
        .await
        .map_or_else(|_| "unread".to_owned(), |v| show(&v));
    let upgrade = strings([
        "upgrade",
        release,
        CHART_REF,
        "--namespace",
        NAMESPACE,
        "--version",
        version,
        "--reuse-values",
        "--set",
        &format!("maxRunners={max_runners}"),
        "--set",
        &format!("minRunners={min_runners}"),
        "--force-conflicts",
    ]);
    let out = tools.run(Tool::Helm, &upgrade).await?;
    if !out.success {
        return Err(format!("helm upgrade: failed\n{}", said(&out))
            .trim_end()
            .to_owned());
    }
    let now = helm_json(tools, &values_argv).await?;
    if held(&now, "maxRunners") != Some(*max_runners)
        || held(&now, "minRunners") != Some(*min_runners)
    {
        return Err(format!(
            "{RUNNER_SCALE}: {release} reads back {}, not maxRunners {max_runners}, minRunners {min_runners}",
            show(&now)
        ));
    }
    Ok(format!(
        "{release}: {} (was {was}; chart {CHART} {version}, read back)",
        show(&now)
    ))
}

#[cfg(test)]
mod tests;
