use super::super::testing::fake;
use super::super::{Action, Output, Tool, carry_out_on, lookup_on};
use super::*;
use serde_json::json;

fn ok(stdout: &str) -> Output {
    Output::new(true, stdout, "")
}

fn doc(params: &serde_json::Value) -> serde_json::Value {
    json!({"action": "disk.prune", "params": params})
}

fn act(target: &str, hours: Option<u64>) -> Action {
    Action::DiskPrune {
        target: target.to_owned(),
        until_hours: hours,
    }
}

/// `docker info`, `df` before, then `rest`, then `df` after.
fn host(before: &str, rest: Vec<Output>, after: &str) -> super::super::testing::Fake {
    let f = fake("", false);
    {
        let mut s = f.script.lock().unwrap();
        s.push_back(ok("/var/lib/docker\n"));
        s.push_back(ok(&format!("Avail\n{before}\n")));
        s.extend(rest);
        s.push_back(ok(&format!("Avail\n{after}\n")));
    }
    f
}

fn argvs(f: &super::super::testing::Fake) -> Vec<String> {
    f.calls()
        .iter()
        .map(|(t, a)| format!("{} {}", t.name(), a.join(" ")))
        .collect()
}

#[test]
fn only_the_closed_targets_are_looked_up() {
    assert!(lookup_on(&doc(&json!({"target": "host"})), "linux").is_ok());
    assert!(
        lookup_on(
            &doc(&json!({"target": "docker-builder", "until_hours": 24})),
            "linux"
        )
        .is_ok()
    );
    for bad in [
        json!({"target": "/mnt/fast"}),
        json!({"target": "all"}),
        json!({"target": "host", "until_hours": 3}),
        json!({"target": "docker-builder"}),
        json!({"target": "docker-builder", "until_hours": 0}),
    ] {
        assert!(lookup_on(&doc(&bad), "linux").is_err(), "{bad}");
    }
    assert!(check("host", None, "macos").unwrap_err().contains("Linux"));
}

#[test]
fn a_k3d_node_name_is_the_sweep_pattern() {
    for good in ["k3d-tbd-server-0", "k3d-a-b-agent-12"] {
        assert!(is_k3d_node(good), "{good}");
    }
    for bad in [
        "k3d-tbd-server",
        "k3d--server-0",
        "tbd-server-0",
        "k3d-Tbd-server-0",
        "k3d-x-proxy-0",
        "k3d-x-server-0\n",
        "k3d-x-server-a",
    ] {
        assert!(!is_k3d_node(bad), "{bad}");
    }
}

#[tokio::test]
async fn host_runs_the_core_task_and_reports_both_free_figures() {
    let f = host("100", vec![ok("")], "250");
    let out = carry_out_on(&act("host", None), &f, "linux").await.unwrap();
    assert!(
        out.contains("100 bytes free before, 250 after, 150 freed"),
        "{out}"
    );
    let calls = argvs(&f);
    assert_eq!(calls[0], "docker info --format {{.DockerRootDir}}");
    assert_eq!(calls[1], "df --output=avail -B1 -- /var/lib/docker");
    assert_eq!(calls[2], "mise run host:prune");
    assert_eq!(f.calls()[2].0, Tool::CoreMise);
    assert_eq!(calls.len(), 4);
}

#[tokio::test]
async fn the_builder_cache_is_cut_by_age_and_nothing_else() {
    let f = host(
        "10",
        vec![Output::new(true, "Total reclaimed space: 1GB\n", "")],
        "20",
    );
    let out = carry_out_on(&act("docker-builder", Some(48)), &f, "linux")
        .await
        .unwrap();
    assert!(out.contains("Total reclaimed space: 1GB"), "{out}");
    assert_eq!(argvs(&f)[2], "docker builder prune -af --filter until=48h");
}

#[tokio::test]
async fn k3d_images_prune_each_running_node_only() {
    let f = host(
        "10",
        vec![
            ok("web\nk3d-tbd-server-0\nk3d-tbd-agent-0\nk3d-tbd-serverlb\n"),
            ok(""),
            ok(""),
        ],
        "11",
    );
    carry_out_on(&act("k3d-images", None), &f, "linux")
        .await
        .unwrap();
    let calls = argvs(&f);
    assert_eq!(calls[2], "docker ps --format {{.Names}}");
    assert_eq!(calls[3], "docker exec k3d-tbd-server-0 crictl rmi --prune");
    assert_eq!(calls[4], "docker exec k3d-tbd-agent-0 crictl rmi --prune");
    assert_eq!(calls.len(), 6);
}

#[tokio::test]
async fn a_run_that_frees_nothing_fails() {
    let f = host("100", vec![ok("")], "100");
    let err = carry_out_on(&act("host", None), &f, "linux")
        .await
        .unwrap_err();
    assert!(
        err.contains("nothing was freed (100 bytes free before, 100 after)"),
        "{err}"
    );
    let f = host("100", vec![ok("")], "90");
    assert!(carry_out_on(&act("host", None), &f, "linux").await.is_err());
}

#[tokio::test]
async fn a_failed_prune_stops_and_says_what_the_tool_said() {
    let f = host("100", vec![Output::new(false, "", "no space left")], "100");
    let err = carry_out_on(&act("host", None), &f, "linux")
        .await
        .unwrap_err();
    assert!(
        err.contains("failed") && err.contains("no space left"),
        "{err}"
    );
    assert_eq!(f.calls().len(), 3, "the after read never runs");
}

#[tokio::test]
async fn k3d_without_a_node_fails_before_deleting() {
    let f = host("100", vec![ok("web\n")], "100");
    let err = carry_out_on(&act("k3d-images", None), &f, "linux")
        .await
        .unwrap_err();
    assert!(err.contains("no k3d node"), "{err}");
}

#[tokio::test]
async fn an_unreadable_df_or_data_root_is_handled() {
    // Docker saying a relative path falls back to the root disk.
    let f = fake("", false);
    {
        let mut s = f.script.lock().unwrap();
        s.push_back(ok("relative\n"));
        s.push_back(ok("Avail\n1\n"));
        s.push_back(ok(""));
        s.push_back(ok("Avail\n2\n"));
    }
    carry_out_on(&act("host", None), &f, "linux").await.unwrap();
    assert_eq!(argvs(&f)[1], "df --output=avail -B1 -- /");
    let f = fake("", false);
    {
        let mut s = f.script.lock().unwrap();
        s.push_back(ok("/var/lib/docker\n"));
        s.push_back(ok("Avail\nnot a number\n"));
    }
    let err = carry_out_on(&act("host", None), &f, "linux")
        .await
        .unwrap_err();
    assert!(err.contains("free bytes"), "{err}");
    assert_eq!(f.calls().len(), 2, "no prune after an unreadable df");
}

#[tokio::test]
async fn a_host_that_is_not_linux_runs_nothing() {
    let f = fake("", false);
    assert!(carry_out_on(&act("host", None), &f, "macos").await.is_err());
    assert!(f.ran_nothing());
}
