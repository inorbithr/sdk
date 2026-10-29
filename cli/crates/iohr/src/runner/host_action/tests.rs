use super::testing::fake;
use super::*;
use serde_json::json;

fn doc(params: &Value) -> Value {
    json!({
        "action": DELETE_FAILED_PODS,
        "runner_id": "rnr_00000000000000000000000001",
        "params": params,
        "effect": "Deletes every failed pod in namespace example.",
    })
}

#[test]
fn the_examples_validate_and_every_invalid_file_is_refused() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/host-action");
    let read = |p: std::path::PathBuf| -> Value {
        serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap()
    };
    let mut n = 0;
    for e in std::fs::read_dir(dir.join("schema/examples")).unwrap() {
        let p = e.unwrap().path();
        assert_eq!(validate(&read(p.clone())), Ok(()), "{}", p.display());
        n += 1;
    }
    assert_eq!(n, 7);
    let mut n = 0;
    for e in std::fs::read_dir(dir.join("schema/invalid")).unwrap() {
        let p = e.unwrap().path();
        assert!(validate(&read(p.clone())).is_err(), "{}", p.display());
        n += 1;
    }
    assert_eq!(n, 24);
}

/// Every vendored file is listed in `SOURCE` with the SHA-256 it was copied with, so a
/// hand edit here, or a file copied without its line, fails.
#[test]
fn the_vendored_fixtures_match_their_source_hashes() {
    fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.push(
                    p.strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/host-action");
    let source = std::fs::read_to_string(dir.join("SOURCE")).unwrap();
    let listed: Vec<(&str, &str)> = source
        .lines()
        .filter_map(|l| l.split_once("  "))
        .filter(|(sha, _)| sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
        .collect();
    for (sha, rel) in &listed {
        let bytes = std::fs::read(dir.join(rel)).unwrap();
        let got = super::super::claude::hex(&<sha2::Sha256 as sha2::Digest>::digest(&bytes));
        assert_eq!(&got, sha, "{rel} differs from what SOURCE says was copied");
    }
    let mut on_disk = Vec::new();
    for sub in ["schema", "vectors"] {
        walk(&dir, &dir.join(sub), &mut on_disk);
    }
    on_disk.sort();
    let mut names: Vec<String> = listed.iter().map(|(_, r)| (*r).to_owned()).collect();
    names.sort();
    assert_eq!(on_disk, names, "SOURCE must list every vendored file once");
}

#[test]
fn the_patterns_hold_at_their_edges() {
    assert!(is_action_id("k8s.delete-failed-pods"));
    assert!(!is_action_id("k8s"));
    assert!(!is_action_id("k8s..x"));
    assert!(!is_action_id("k8s.-x"));
    assert!(!is_action_id("K8s.x"));
    assert!(!is_action_id(&format!("a.{}", "b".repeat(63))));
    assert!(is_runner_id("rnr_01M5Z0WF3MRKDQ75T7F72E3F9N"));
    assert!(!is_runner_id("rnr_01m5z0wf3mrkdq75t7f72e3f9n"));
    assert!(!is_runner_id("rnr_01M5Z0WF3MRKDQ75T7F72E3F9"));
    assert!(is_dns_label("a"));
    assert!(is_dns_label("tbd-dev"));
    assert!(is_dns_label(&"a".repeat(63)));
    assert!(!is_dns_label(&"a".repeat(64)));
    assert!(!is_dns_label("-a"));
    assert!(!is_dns_label("a-"));
    assert!(!is_dns_label("A"));
    assert!(!is_dns_label("example; rm -rf /"));
    assert!(!is_dns_label(""));
}

#[test]
fn an_id_outside_the_catalog_is_an_unknown_action() {
    let mut d = doc(&json!({}));
    d["action"] = json!("k8s.delete-namespace");
    assert_eq!(validate(&d), Err("/action: unknown action".to_owned()));
    assert_eq!(lookup(&d), Err("unknown action".to_owned()));
    assert_eq!(
        lookup(&doc(&json!({"namespace": "example"}))),
        Ok(Action::DeleteFailedPods {
            namespace: Some("example".to_owned())
        })
    );
}

#[test]
fn the_effect_is_the_catalog_sentence_for_each_action_and_its_params() {
    let cases = [
        (
            DELETE_FAILED_PODS,
            json!({}),
            "Deletes every failed pod in every namespace.",
        ),
        (
            DELETE_FAILED_PODS,
            json!({"namespace": "example"}),
            "Deletes every failed pod in namespace example.",
        ),
        (
            ROLLOUT_RESTART,
            json!({"namespace": "ns", "deployment": "web"}),
            "Restarts deployment web in namespace ns and waits for the new pods.",
        ),
        (
            USER_RESTART,
            json!({"unit": "iohr-runner.service"}),
            "Restarts the user service iohr-runner.service on the runner's host.",
        ),
        (
            PRUNE_IMAGES,
            json!({"until_hours": 24}),
            "Removes the unused Docker images older than 24 hours on the runner's host.",
        ),
        (
            CANCEL_RUN,
            json!({"run_id": 7, "repo": "inorbithr/core"}),
            "Cancels GitHub Actions run 7 of inorbithr/core.",
        ),
        (
            UPGRADE,
            json!({}),
            "Upgrades iohr on the runner's host to the newest release.",
        ),
    ];
    for (id, params, want) in cases {
        assert_eq!(effect_of(id, &params).as_deref(), Some(want), "{id}");
        assert_eq!(validate(&action(id, &params)), Ok(()), "{id}");
    }
}

#[test]
fn a_run_whose_effect_is_not_the_sentence_for_its_params_is_refused() {
    let refused =
        Err("/effect: not the catalog's sentence for the action and its params".to_owned());
    // Free text, another action's sentence, the same sentence for other params, and a
    // sentence with a trailing change: each is a well-formed effect that is not this run's.
    let mut d = action(DELETE_FAILED_PODS, &json!({"namespace": "example"}));
    for effect in [
        "Does one thing on the runner's host.",
        "Deletes every failed pod in every namespace.",
        "Deletes every failed pod in namespace other.",
        "Deletes every failed pod in namespace example. ",
    ] {
        d["effect"] = json!(effect);
        assert_eq!(validate(&d), refused, "{effect}");
    }
    let mut d = action(PRUNE_IMAGES, &json!({"until_hours": 24}));
    d["params"] = json!({"until_hours": 48});
    assert_eq!(validate(&d), refused);
}

#[test]
fn an_action_outside_the_catalog_has_no_effect_and_a_wrong_param_type_has_none_either() {
    assert_eq!(effect_of("sh.run", &json!({})), None);
    assert_eq!(
        effect_of("k8s.delete-namespace", &json!({"namespace": "example"})),
        None
    );
    for (id, params) in [
        (DELETE_FAILED_PODS, json!({"namespace": 7})),
        (ROLLOUT_RESTART, json!({"namespace": "ns"})),
        (ROLLOUT_RESTART, json!({"namespace": "ns", "deployment": 1})),
        (USER_RESTART, json!({"unit": ["a"]})),
        (PRUNE_IMAGES, json!({"until_hours": "24"})),
        (PRUNE_IMAGES, json!({"until_hours": 24.5})),
        (CANCEL_RUN, json!({"run_id": "7", "repo": "inorbithr/core"})),
        (CANCEL_RUN, json!({"run_id": 7, "repo": 3})),
    ] {
        assert_eq!(effect_of(id, &params), None, "{id} {params}");
        assert!(validate(&action(id, &params)).is_err(), "{id} {params}");
    }
    // An unknown action is refused whatever effect it carries.
    let mut d = action(UPGRADE, &json!({}));
    d["action"] = json!("sh.run");
    assert!(validate(&d).is_err());
}

#[test]
fn the_cluster_namespaces_and_bad_names_are_refused() {
    for ns in DENIED_NAMESPACES {
        assert!(delete_failed_pods_argv(ns).is_err(), "{ns}");
    }
    assert!(delete_failed_pods_argv("example; rm -rf /").is_err());
    assert!(delete_failed_pods_argv("$(id)").is_err());
    assert!(delete_failed_pods_argv("").is_err());
}

#[test]
fn the_argv_is_kubectl_arguments_with_no_shell() {
    let argv = delete_failed_pods_argv("example").unwrap();
    assert_eq!(
        argv,
        [
            "delete",
            "pods",
            "--field-selector=status.phase=Failed",
            "-n",
            "example"
        ]
    );
    // No shell between us and kubectl: no `sh`, no `-c`, nothing a shell would read.
    assert!(argv.iter().all(|a| a != "sh" && a != "-c" && a != "bash"));
    assert!(
        argv.iter()
            .all(|a| !a.contains([';', '|', '&', '$', '`', ' ', '\n']))
    );
}

#[tokio::test]
async fn every_namespace_skips_the_cluster_s_own() {
    let k = fake(
        "default kube-system tbd kube-public kube-node-lease BAD",
        false,
    );
    let said = carry_out(&Action::DeleteFailedPods { namespace: None }, &k)
        .await
        .unwrap();
    let calls = k.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0], (Tool::Kubectl, list_namespaces_argv()));
    assert_eq!(calls[1].1[4], "default");
    assert_eq!(calls[2].1[4], "tbd");
    assert!(said.contains("default:") && said.contains("tbd:"));
}

#[tokio::test]
async fn a_named_cluster_namespace_runs_nothing() {
    let k = fake("", false);
    let r = carry_out(
        &Action::DeleteFailedPods {
            namespace: Some("kube-system".to_owned()),
        },
        &k,
    )
    .await;
    assert!(r.is_err());
    assert!(k.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_refused_delete_is_a_failure_with_kubectl_s_words() {
    let k = fake("", true);
    let r = carry_out(
        &Action::DeleteFailedPods {
            namespace: Some("example".to_owned()),
        },
        &k,
    )
    .await;
    assert_eq!(r, Err("example: forbidden".to_owned()));
}

#[test]
fn no_configured_cluster_is_refused_and_a_flag_is_not_a_context() {
    assert!(KubeTarget::new(None, None).is_err());
    assert!(KubeTarget::new(Some("  "), Some("")).is_err());
    assert!(HostRunner::find(None, None, &[]).kube_problem().is_some());
    assert!(KubeTarget::new(Some("--server=https://x"), None).is_err());
    assert!(KubeTarget::new(None, Some("relative/config")).is_err());
    assert!(KubeTarget::new(None, Some("-/etc/x")).is_err());
    // An absolute path on every platform ("/etc/..." is not one on Windows).
    let config = std::env::temp_dir().join("k").join("config");
    let config = config.to_str().unwrap();
    let both = KubeTarget::new(Some("prod"), Some(config)).unwrap();
    assert_eq!(both.args(), ["--kubeconfig", config, "--context", "prod"]);
    assert_eq!(
        KubeTarget::new(Some(" prod "), None).unwrap().args(),
        ["--context", "prod"]
    );
}

/// A `kubectl` that writes its arguments to a file, so the real process path is seen.
#[cfg(unix)]
#[tokio::test]
async fn every_call_carries_the_pinned_context_before_its_own_arguments() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("args");
    let script = dir.path().join("kubectl");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n", out.display()),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let k = host(dir.path(), Ok(KubeTarget::new(Some("prod"), None).unwrap()));
    let r = k
        .run(Tool::Kubectl, &delete_failed_pods_argv("example").unwrap())
        .await
        .unwrap();
    assert!(r.success);
    let got = std::fs::read_to_string(&out).unwrap();
    let args: Vec<&str> = got.lines().collect();
    assert_eq!(&args[..3], ["--context", "prod", "delete"]);
    assert_eq!(args.last().copied(), Some("example"));
}

#[test]
fn what_kubectl_says_is_redacted_before_it_is_reported() {
    let said = redact_output(
        "Unable to connect to the server: dial tcp 10.0.0.5:6443: i/o timeout\n\
         error: You must be logged in (https://api.example.com:6443/api) Bearer abcdef\n\
         token ghp_abcdefghijklmnopqrstuvwxyz0123456789 eyJhbGciOiJIUzI1NiJ9.e30.sig\n\
         pods is forbidden in namespace \"example\"",
    );
    for leak in [
        "10.0.0.5",
        "6443",
        "example.com",
        "https://",
        "abcdef",
        "ghp_",
        "eyJ",
    ] {
        assert!(!said.contains(leak), "{leak} leaked: {said}");
    }
    assert!(said.contains("forbidden in namespace \"example\""));
    assert_eq!(said.lines().count(), 4);
}

#[tokio::test]
async fn a_refused_delete_does_not_report_a_server_address() {
    let k = Leaky;
    let r = carry_out(
        &Action::DeleteFailedPods {
            namespace: Some("example".to_owned()),
        },
        &k,
    )
    .await
    .unwrap_err();
    assert!(!r.contains("10.1.2.3") && !r.contains("https://"), "{r}");
    assert!(r.contains("[redacted]"));
}

struct Leaky;

#[allow(
    unknown_lints,
    clippy::unused_async_trait_impl,
    reason = "a fake answers at once; the trait is async for the real tools"
)]
impl ArgvRunner for Leaky {
    async fn run(&self, _tool: Tool, _argv: &[String]) -> Result<Output, String> {
        Ok(Output::new(
            false,
            "",
            "Get https://10.1.2.3:6443/api: connection refused",
        ))
    }
}

/// A host whose `PATH` is `dir` alone, with no environment of its own.
fn host(dir: &std::path::Path, kube: Result<KubeTarget, String>) -> HostRunner {
    HostRunner {
        path: Some(dir.as_os_str().to_owned()),
        env: Vec::new(),
        kube,
        iohr: None,
        install_env: Vec::new(),
        user_units: vec!["iohr-runner.service".to_owned()],
    }
}

/// A run of `id` whose `effect` is the catalog's sentence for it (the words the owner
/// approves), as the coordinator files it. Params the catalog has no sentence for (a
/// run that is refused anyway) carry a one-line placeholder, so the refusal under test
/// is the params' own.
fn action(id: &str, params: &Value) -> Value {
    json!({
        "action": id,
        "runner_id": "rnr_00000000000000000000000001",
        "params": params,
        "effect": effect_of(id, params).unwrap_or_else(|| "Placeholder.".to_owned()),
    })
}

type Calls = Vec<(Tool, Vec<String>)>;

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn the_new_patterns_hold_at_their_edges() {
    assert!(is_user_unit("iohr-runner.service"));
    assert!(is_user_unit("getty@tty1.service"));
    assert!(is_user_unit(&format!("{}.service", "a".repeat(63))));
    assert!(!is_user_unit(&format!("{}.service", "a".repeat(64))));
    assert!(!is_user_unit(".service"));
    assert!(!is_user_unit("-x.service"));
    assert!(!is_user_unit("example.timer"));
    assert!(!is_user_unit("x;reboot.service"));
    assert!(!is_user_unit("X.service"));
    assert!(is_repo("inorbithr/core"));
    assert!(is_repo(&format!("inorbithr/{}", "a".repeat(100))));
    assert!(!is_repo(&format!("inorbithr/{}", "a".repeat(101))));
    assert!(!is_repo("inorbithr/"));
    assert!(!is_repo("example/example"));
    assert!(!is_repo("inorbithr/core/x"));
    assert!(!is_repo("inorbithr/Core"));
    assert!(!is_repo("inorbithr/."));
    assert!(!is_repo("inorbithr/.."));
    assert!(is_repo("inorbithr/.github"));
}

#[test]
fn every_new_action_reads_its_typed_params() {
    let cases = [
        (
            action(
                ROLLOUT_RESTART,
                &json!({"namespace": "tbd", "deployment": "gateway"}),
            ),
            Action::RolloutRestart {
                namespace: "tbd".to_owned(),
                deployment: "gateway".to_owned(),
            },
        ),
        (
            action(USER_RESTART, &json!({"unit": "iohr-runner.service"})),
            Action::UserRestart {
                unit: "iohr-runner.service".to_owned(),
            },
        ),
        (
            action(PRUNE_IMAGES, &json!({"until_hours": 24})),
            Action::PruneImages { until_hours: 24 },
        ),
        (
            action(CANCEL_RUN, &json!({"repo": "inorbithr/core", "run_id": 7})),
            Action::CancelRun {
                repo: "inorbithr/core".to_owned(),
                run_id: 7,
            },
        ),
        (action(UPGRADE, &json!({})), Action::Upgrade),
    ];
    for (doc, want) in cases {
        assert_eq!(validate(&doc), Ok(()), "{doc}");
        assert_eq!(lookup_on(&doc, "linux"), Ok(want.clone()));
        assert_eq!(want.id(), doc["action"]);
    }
}

#[test]
fn a_bad_integer_another_owner_or_an_extra_param_is_refused() {
    for (id, params) in [
        (PRUNE_IMAGES, json!({"until_hours": 0})),
        (PRUNE_IMAGES, json!({"until_hours": 721})),
        (PRUNE_IMAGES, json!({"until_hours": 24.0})),
        (PRUNE_IMAGES, json!({"until_hours": -1})),
        (PRUNE_IMAGES, json!({"until_hours": "24"})),
        (PRUNE_IMAGES, json!({})),
        (CANCEL_RUN, json!({"repo": "example/example", "run_id": 1})),
        (CANCEL_RUN, json!({"repo": "inorbithr/core", "run_id": 0})),
        (
            CANCEL_RUN,
            json!({"repo": "inorbithr/core", "run_id": 9_007_199_254_740_992_u64}),
        ),
        (CANCEL_RUN, json!({"repo": "inorbithr/core"})),
        (UPGRADE, json!({"version": "1.0.0"})),
        (USER_RESTART, json!({"unit": "x;reboot.service"})),
        (ROLLOUT_RESTART, json!({"namespace": "tbd"})),
        (
            ROLLOUT_RESTART,
            json!({"namespace": "tbd", "deployment": "a", "replicas": 1}),
        ),
    ] {
        let doc = action(id, &params);
        let why = validate(&doc).unwrap_err();
        // The error names where, never the value.
        assert!(why.starts_with("/params"), "{id} {params}: {why}");
        assert!(
            !why.contains("reboot") && !why.contains("example/"),
            "{why}"
        );
    }
}

#[tokio::test]
async fn each_action_runs_exactly_its_argv() {
    let cases: [(Action, Calls); 4] = [
        (
            Action::RolloutRestart {
                namespace: "tbd".to_owned(),
                deployment: "gateway".to_owned(),
            },
            vec![
                (
                    Tool::Kubectl,
                    strs(&["rollout", "restart", "deployment/gateway", "-n", "tbd"]),
                ),
                (
                    Tool::Kubectl,
                    strs(&[
                        "rollout",
                        "status",
                        "deployment/gateway",
                        "-n",
                        "tbd",
                        "--timeout=240s",
                    ]),
                ),
            ],
        ),
        (
            Action::UserRestart {
                unit: "iohr-runner.service".to_owned(),
            },
            vec![
                (Tool::Systemctl, strs(&["--user", "daemon-reload"])),
                (
                    Tool::Systemctl,
                    strs(&["--user", "restart", "iohr-runner.service"]),
                ),
            ],
        ),
        (
            Action::PruneImages { until_hours: 24 },
            vec![(
                Tool::Docker,
                strs(&["image", "prune", "-af", "--filter", "until=24h"]),
            )],
        ),
        (
            Action::CancelRun {
                repo: "inorbithr/core".to_owned(),
                run_id: 123,
            },
            vec![(
                Tool::Gh,
                strs(&["run", "cancel", "123", "-R", "inorbithr/core"]),
            )],
        ),
    ];
    for (a, want) in cases {
        let k = fake("", false);
        carry_out_on(&a, &k, "linux").await.unwrap();
        assert_eq!(k.calls(), want, "{a:?}");
        // No shell anywhere, and no prune of anything but images.
        for (_, argv) in &want {
            assert!(argv.iter().all(|x| x != "sh" && x != "-c" && x != "system"));
            assert!(argv.iter().all(|x| x != "volume" && x != "--volumes"));
        }
    }
    let k = fake("", false);
    let said = carry_out_on(&Action::Upgrade, &k, "macos").await.unwrap();
    assert_eq!(k.calls(), [(Tool::Iohr, strs(&["upgrade"]))]);
    assert_eq!(said, "iohr upgrade: done");
}

#[tokio::test]
async fn a_cluster_namespace_restart_runs_nothing() {
    for ns in DENIED_NAMESPACES {
        let doc = action(
            ROLLOUT_RESTART,
            &json!({"namespace": ns, "deployment": "coredns"}),
        );
        assert_eq!(validate(&doc), Ok(()));
        assert!(
            lookup_on(&doc, "linux")
                .unwrap_err()
                .contains("cluster's own")
        );
        let k = fake("", false);
        let a = Action::RolloutRestart {
            namespace: (*ns).to_owned(),
            deployment: "coredns".to_owned(),
        };
        assert!(carry_out_on(&a, &k, "linux").await.is_err());
        assert!(k.ran_nothing());
    }
}

#[tokio::test]
async fn a_user_restart_off_linux_is_refused_before_it_runs() {
    let doc = action(USER_RESTART, &json!({"unit": "iohr-runner.service"}));
    let why = lookup_on(&doc, "macos").unwrap_err();
    assert!(
        why.contains("only a Linux runner") && why.contains("macos"),
        "{why}"
    );
    let k = fake("", false);
    let a = Action::UserRestart {
        unit: "iohr-runner.service".to_owned(),
    };
    assert!(carry_out_on(&a, &k, "windows").await.is_err());
    assert!(k.ran_nothing());
}

#[tokio::test]
async fn only_the_user_units_the_runner_lists_are_restarted() {
    let dir = tempfile::tempdir().unwrap();
    let h = host(dir.path(), Err("none".to_owned()));
    let restart = |unit: &str| Action::UserRestart {
        unit: unit.to_owned(),
    };
    assert_eq!(h.refuses(&restart("iohr-runner.service")), None);
    let why = h.refuses(&restart("sshd.service")).unwrap();
    assert!(why.contains("not in this runner's user_units"), "{why}");
    // `run` holds to the list too, whoever calls it.
    let ran = h
        .run(
            Tool::Systemctl,
            &strs(&["--user", "restart", "sshd.service"]),
        )
        .await;
    assert_eq!(ran, Err(why));
    // No list: every unit is refused.
    let none = HostRunner {
        user_units: Vec::new(),
        ..h
    };
    let why = none.refuses(&restart("iohr-runner.service")).unwrap();
    assert!(why.contains("lists no user_units"), "{why}");
    assert_eq!(none.refuses(&Action::PruneImages { until_hours: 24 }), None);
}

#[tokio::test]
async fn a_failed_call_stops_the_action_with_its_words() {
    let k = fake("", true);
    let a = Action::UserRestart {
        unit: "iohr-runner.service".to_owned(),
    };
    let why = carry_out_on(&a, &k, "linux").await.unwrap_err();
    assert_eq!(why, "systemctl --user daemon-reload: failed\nforbidden");
    // The restart never ran after the reload failed.
    assert_eq!(k.calls().len(), 1);
}

#[tokio::test]
async fn a_missing_tool_is_a_refusal_and_nothing_runs() {
    let dir = tempfile::tempdir().unwrap();
    let h = host(dir.path(), Err("no kube context is configured".to_owned()));
    for (tool, want) in [
        (Tool::Docker, "docker is not on this runner's PATH"),
        (Tool::Gh, "gh is not on this runner's PATH"),
        (Tool::Systemctl, "systemctl is not on this runner's PATH"),
        (Tool::Kubectl, "no kube context is configured"),
        (Tool::Iohr, "this runner cannot tell where its iohr is"),
    ] {
        assert_eq!(h.run(tool, &strs(&["x"])).await, Err(want.to_owned()));
    }
}

#[tokio::test]
async fn the_prune_report_counts_images_and_space() {
    let mut k = fake("", false);
    k.answer = Some(Output::new(
        true,
        "Deleted Images:\nuntagged: example:1\ndeleted: sha256:aaaa\ndeleted: sha256:bbbb\n\nTotal reclaimed space: 1.2GB\n",
        "",
    ));
    let said = carry_out_on(&Action::PruneImages { until_hours: 24 }, &k, "linux")
        .await
        .unwrap();
    assert_eq!(said, "2 images removed; Total reclaimed space: 1.2GB");
}

#[tokio::test]
async fn every_tool_s_output_is_redacted_and_capped() {
    let actions = [
        Action::RolloutRestart {
            namespace: "tbd".to_owned(),
            deployment: "gateway".to_owned(),
        },
        Action::UserRestart {
            unit: "iohr-runner.service".to_owned(),
        },
        Action::CancelRun {
            repo: "inorbithr/core".to_owned(),
            run_id: 1,
        },
        Action::Upgrade,
    ];
    for success in [true, false] {
        for a in &actions {
            let mut k = fake("", false);
            k.answer = Some(Output::new(
                success,
                "fetched https://10.1.2.3:6443/x with ghp_abcdefghijklmnopqrstuvwxyz0123456789",
                "Bearer\tabcdef",
            ));
            let said = match carry_out_on(a, &k, "linux").await {
                Ok(s) | Err(s) => s,
            };
            for leak in ["10.1.2.3", "https://", "ghp_", "abcdef"] {
                assert!(!said.contains(leak), "{a:?}: {leak} leaked: {said}");
            }
            assert!(said.contains("[redacted]"), "{a:?}: {said}");
        }
    }
    let mut k = fake("", false);
    k.answer = Some(Output::new(
        true,
        &"é".repeat(60 * 1024),
        &"x ".repeat(30 * 1024),
    ));
    let said = carry_out_on(&Action::Upgrade, &k, "linux").await.unwrap();
    assert!(said.len() <= OUTPUT_CAP, "{}", said.len());
}

#[test]
fn a_bearer_value_after_any_whitespace_is_redacted() {
    for text in [
        "Bearer\tabcdef",
        "Bearer  abcdef",
        "token:\u{a0}abcdef",
        "x Bearer abcdef y",
    ] {
        let said = redact_output(text);
        assert!(!said.contains("abcdef"), "{text:?}: {said}");
        assert!(said.contains("[redacted]"), "{text:?}: {said}");
    }
    assert_eq!(redact_output("a\tb  c"), "a\tb  c");
}

#[test]
fn a_cut_at_the_cap_never_leaves_half_a_word() {
    // A token that crosses the cap is dropped whole, not left as a short fragment that
    // no longer looks secret.
    let pad = "a ".repeat((OUTPUT_CAP - 10) / 2);
    let text = format!("{pad}ghp_abcdefghijklmnopqrstuvwxyz0123456789 end");
    let out = Output::new(true, &text, "");
    assert!(out.stdout.len() <= OUTPUT_CAP);
    assert!(
        out.stdout.ends_with("a "),
        "{:?}",
        &out.stdout[out.stdout.len() - 20..]
    );
    assert!(!out.stdout.contains("abcdef"));
}

/// A `docker` that prunes far more than [`OUTPUT_CAP`] of images, on both streams: the
/// count and the total come from the whole output, the report stays capped.
#[cfg(unix)]
#[tokio::test]
async fn a_large_prune_is_counted_from_the_whole_output() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("docker");
    std::fs::write(
        &script,
        concat!(
            "#!/bin/sh\n",
            "echo 'Deleted Images:'\n",
            "i=0\n",
            "while [ $i -lt 2000 ]; do\n",
            "  printf 'untagged: example:%d\\ndeleted: sha256:%064d\\n' $i $i\n",
            "  printf 'warning %d\\n' $i >&2\n",
            "  i=$((i+1))\n",
            "done\n",
            "echo\n",
            "echo 'Total reclaimed space: 9.9GB'\n",
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let h = host(dir.path(), Err("none".to_owned()));
    let out = h
        .run(
            Tool::Docker,
            &plan(&Action::PruneImages { until_hours: 24 }, "linux").unwrap()[0].1,
        )
        .await
        .unwrap();
    assert!(out.stdout.len() <= OUTPUT_CAP && out.stderr.len() <= OUTPUT_CAP);
    assert_eq!(out.tally.deleted, 2000);
    let said = carry_out_on(&Action::PruneImages { until_hours: 24 }, &h, "linux")
        .await
        .unwrap();
    assert_eq!(said, "2000 images removed; Total reclaimed space: 9.9GB");
}

/// A `gh` that writes its environment to a file: it is asked never to prompt, and gets
/// no token from the runner's own environment.
#[cfg(unix)]
#[tokio::test]
async fn gh_runs_without_prompts_and_without_the_runner_s_token() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("env");
    let script = dir.path().join("gh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nenv > '{}'\nprintf '%s\\n' \"$@\" >> '{}'\n",
            out.display(),
            out.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut h = host(dir.path(), Err("none".to_owned()));
    // What env::scrub would have kept, plus what it drops, to prove the runner's list
    // is the one passed.
    h.env = env::scrub(
        [
            ("HOME", "/home/example"),
            ("GH_TOKEN", "ghp_abcdefghijklmnopqrstuvwxyz0123456789"),
        ]
        .into_iter()
        .map(|(k, v)| (OsString::from(k), OsString::from(v))),
    );
    let said = carry_out_on(
        &Action::CancelRun {
            repo: "inorbithr/core".to_owned(),
            run_id: 9,
        },
        &h,
        "linux",
    )
    .await
    .unwrap();
    assert_eq!(said, "inorbithr/core run 9: cancel requested");
    let got = std::fs::read_to_string(&out).unwrap();
    assert!(got.contains("GH_PROMPT_DISABLED=1"), "{got}");
    assert!(got.contains("HOME=/home/example"), "{got}");
    assert!(!got.contains("GH_TOKEN"), "{got}");
    assert!(
        got.ends_with("run\ncancel\n9\n-R\ninorbithr/core\n"),
        "{got}"
    );
}

/// A tool that leaves a child behind: dropping its run (as a timeout does) kills that child too,
/// because the tool runs in its own process group.
#[cfg(unix)]
#[tokio::test]
async fn a_dropped_run_takes_what_its_tool_started() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("pid");
    let script = dir.path().join("gh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n/bin/sleep 30 &\necho $! > '{}'\nwait\n",
            pidfile.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let k = host(dir.path(), Err("no cluster".to_owned()));
    let run = tokio::spawn(async move { k.run(Tool::Gh, &strs(&["run", "list"])).await });
    let mut pid = None;
    for _ in 0..100 {
        if let Some(p) = std::fs::read_to_string(&pidfile)
            .ok()
            .and_then(|t| t.trim().parse::<i32>().ok())
        {
            pid = Some(p);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let pid = pid.unwrap();
    run.abort();
    let _ = run.await;
    assert!(crate::runner::exec::process_gone(pid).await);
}
