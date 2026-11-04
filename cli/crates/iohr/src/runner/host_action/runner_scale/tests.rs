use super::super::testing::{Fake, fake};
use super::super::{Action, Output, Tool, carry_out_on, lookup_on};
use super::*;
use serde_json::json;

fn ok(stdout: &str) -> Output {
    Output::new(true, stdout, "")
}

fn act(release: &str, max: u64, min: u64) -> Action {
    Action::RunnerScale {
        release: release.to_owned(),
        max_runners: max,
        min_runners: min,
    }
}

fn meta(chart: &str, version: &str) -> Output {
    ok(
        &json!({"name": "core-ci", "chart": chart, "version": version, "namespace": "arc-runners"})
            .to_string(),
    )
}

fn values(max: u64, min: u64) -> Output {
    ok(
        &json!({"maxRunners": max, "minRunners": min, "githubConfigUrl": "https://github.com/x"})
            .to_string(),
    )
}

/// metadata, values before, upgrade, values after.
fn host(steps: Vec<Output>) -> Fake {
    let f = fake("", false);
    f.script.lock().unwrap().extend(steps);
    f
}

fn argvs(f: &Fake) -> Vec<String> {
    f.calls()
        .iter()
        .map(|(t, a)| format!("{} {}", t.name(), a.join(" ")))
        .collect()
}

#[test]
fn only_the_catalog_releases_and_counts_are_looked_up() {
    let doc = |p: serde_json::Value| json!({"action": "helm.runner-scale", "params": p});
    assert_eq!(
        lookup_on(
            &doc(json!({"release": "core-ci", "max_runners": 4})),
            "linux"
        )
        .unwrap(),
        act("core-ci", 4, 0)
    );
    assert_eq!(
        lookup_on(
            &doc(json!({"release": "core-images", "max_runners": 8, "min_runners": 2})),
            "macos"
        )
        .unwrap(),
        act("core-images", 8, 2)
    );
    for bad in [
        json!({"release": "prod", "max_runners": 4}),
        json!({"release": "core-ci", "max_runners": 0}),
        json!({"release": "core-ci", "max_runners": 33}),
        json!({"release": "core-ci", "max_runners": 2, "min_runners": 3}),
        json!({"release": "core-ci"}),
    ] {
        assert!(lookup_on(&doc(bad.clone()), "linux").is_err(), "{bad}");
    }
}

#[tokio::test]
async fn a_scale_reuses_values_pins_the_chart_version_and_reads_back() {
    let f = host(vec![
        meta(CHART, "0.13.1"),
        values(8, 1),
        ok("Release upgraded"),
        values(4, 0),
    ]);
    let out = carry_out_on(&act("core-ci", 4, 0), &f, "linux")
        .await
        .unwrap();
    assert!(
        out.contains("core-ci: maxRunners 4, minRunners 0 (was maxRunners 8, minRunners 1;"),
        "{out}"
    );
    let calls = argvs(&f);
    assert_eq!(calls[0], "helm get metadata core-ci -n arc-runners -o json");
    assert_eq!(
        calls[2],
        format!(
            "helm upgrade core-ci {CHART_REF} --namespace arc-runners --version 0.13.1 --reuse-values --set maxRunners=4 --set minRunners=0 --force-conflicts"
        )
    );
    assert!(f.calls().iter().all(|(t, _)| *t == Tool::Helm));
    assert_eq!(calls.len(), 4);
}

#[tokio::test]
async fn a_value_that_does_not_read_back_fails() {
    let f = host(vec![
        meta(CHART, "0.13.1"),
        values(8, 1),
        ok(""),
        values(8, 0),
    ]);
    let err = carry_out_on(&act("core-ci", 4, 0), &f, "linux")
        .await
        .unwrap_err();
    assert!(
        err.contains("reads back maxRunners 8, minRunners 0"),
        "{err}"
    );
}

#[tokio::test]
async fn another_chart_or_a_bad_version_is_refused_before_the_upgrade() {
    let f = host(vec![meta("nginx", "0.13.1")]);
    let err = carry_out_on(&act("core-ci", 4, 0), &f, "linux")
        .await
        .unwrap_err();
    assert!(err.contains("not a gha-runner-scale-set release"), "{err}");
    assert_eq!(f.calls().len(), 1);
    let f = host(vec![meta(CHART, "--set x=1")]);
    assert!(
        carry_out_on(&act("core-ci", 4, 0), &f, "linux")
            .await
            .is_err()
    );
    assert_eq!(f.calls().len(), 1);
}

#[tokio::test]
async fn a_failed_upgrade_says_what_helm_said() {
    let f = host(vec![
        meta(CHART, "0.13.1"),
        values(8, 1),
        Output::new(false, "", "Error: conflict"),
    ]);
    let err = carry_out_on(&act("core-ci", 4, 0), &f, "linux")
        .await
        .unwrap_err();
    assert!(
        err.contains("helm upgrade: failed") && err.contains("conflict"),
        "{err}"
    );
    assert_eq!(f.calls().len(), 3);
}

#[tokio::test]
async fn a_refused_scale_runs_nothing() {
    for a in [
        act("prod", 4, 0),
        act("core-ci", 33, 0),
        act("core-ci", 2, 3),
    ] {
        let f = fake("", false);
        assert!(carry_out_on(&a, &f, "linux").await.is_err());
        assert!(f.ran_nothing());
    }
}

#[test]
fn a_chart_version_is_three_numbers() {
    assert!(is_chart_version("0.13.1"));
    for bad in ["", "1.2", "1.2.3.4", "v1.2.3", "1.2.x", "-1.2.3"] {
        assert!(!is_chart_version(bad), "{bad}");
    }
}
