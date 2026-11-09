use super::super::testing::{Fake, fake};
use super::super::{Action, Output, carry_out_on, lookup_on, validate};
use super::*;
use serde_json::json;

const SITES: &str = "inorbit.hr {\n\timport site\n}\n";

fn ok(stdout: &str) -> Output {
    Output::new(true, stdout, "")
}

fn failed(stderr: &str) -> Output {
    Output::new(false, "", stderr)
}

fn unauthorized() -> Output {
    failed("  HTTP/1.1 401 Unauthorized\nwget: server returned error: HTTP/1.1 401 Unauthorized")
}

/// An edge folder with a Caddyfile that defines `(api)` and the site file.
fn edge(sites: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Caddyfile"),
        "(api) {\n\treverse_proxy api:8080\n}\nimport sites.d/*.caddy\n",
    )
    .unwrap();
    std::fs::create_dir(dir.path().join("sites.d")).unwrap();
    std::fs::write(dir.path().join(SITE_FILE), sites).unwrap();
    dir
}

fn inspect(dir: &Path) -> Output {
    ok(&format!("true\t{}\n", dir.display()))
}

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

fn backups(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir.join("sites.d"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n != "inorbit.hr.caddy")
        .collect();
    names.sort();
    names
}

#[test]
fn it_takes_no_params_and_runs_on_linux_only() {
    let doc = |p: serde_json::Value| json!({"action": "edge.caddy-site-call", "params": p});
    assert_eq!(
        lookup_on(&doc(json!({})), "linux").unwrap(),
        Action::EdgeSiteCall
    );
    let signed = |p: serde_json::Value| {
        json!({
            "action": "edge.caddy-site-call",
            "runner_id": "rnr_00000000000000000000000001",
            "params": p,
            "effect": EFFECT,
        })
    };
    assert!(validate(&signed(json!({}))).is_ok());
    assert!(validate(&signed(json!({"site": "evil.example"}))).is_err());
    assert!(check("linux").is_ok());
    assert!(check("macos").unwrap_err().contains("Linux only"));
}

#[test]
fn the_block_goes_after_a_blank_line_and_the_mount_must_be_a_plain_folder() {
    assert_eq!(appended(""), BLOCK);
    assert_eq!(appended("a {\n}"), format!("a {{\n}}\n\n{BLOCK}"));
    assert_eq!(appended("a {\n}\n"), format!("a {{\n}}\n\n{BLOCK}"));
    assert_eq!(
        edge_dir("true\t/srv/edge\n").unwrap(),
        PathBuf::from("/srv/edge")
    );
    assert!(
        edge_dir("false\t/srv/edge")
            .unwrap_err()
            .contains("not running")
    );
    for bad in ["true\t", "true\tedge", "true\t/srv/../etc", "true"] {
        assert!(
            edge_dir(bad).unwrap_err().contains("no host folder"),
            "{bad}"
        );
    }
    assert!(answered_401("  HTTP/1.1 401 Unauthorized"));
    assert!(!answered_401(
        "  HTTP/1.1 525 <none>\nwget: server returned error: HTTP/1.1 525"
    ));
    assert!(!answered_401(""));
}

#[tokio::test]
async fn it_adds_the_site_after_a_backup_validates_reloads_and_waits_for_401() {
    let dir = edge(SITES);
    let f = host(vec![
        inspect(dir.path()),
        ok(""),
        ok(""),
        failed("  HTTP/1.1 525 <none>"),
        unauthorized(),
    ]);
    let out = carry_out_on(&Action::EdgeSiteCall, &f, "linux")
        .await
        .unwrap();
    assert!(out.contains("added to sites.d/inorbit.hr.caddy"), "{out}");
    assert!(out.contains("answers 401"), "{out}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join(SITE_FILE)).unwrap(),
        format!("{SITES}\n{BLOCK}")
    );
    let kept = backups(dir.path());
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert!(kept[0].starts_with("inorbit.hr.caddy.bak-"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("sites.d").join(&kept[0])).unwrap(),
        SITES
    );
    let calls = argvs(&f);
    assert_eq!(calls.len(), 5);
    assert!(calls[0].starts_with("docker inspect --format "));
    assert!(calls[0].ends_with(" tbd-edge-caddy-1"));
    assert_eq!(
        calls[1],
        "docker exec tbd-edge-caddy-1 caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile"
    );
    assert_eq!(
        calls[2],
        "docker exec tbd-edge-caddy-1 caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile"
    );
    assert_eq!(
        calls[4],
        "docker exec tbd-edge-caddy-1 wget -S -q -O /dev/null -T 5 https://call.inorbit.hr/rtc/validate"
    );
}

#[tokio::test]
async fn a_site_already_there_is_left_alone_and_still_reloaded() {
    let with = format!("{SITES}\n{BLOCK}");
    let dir = edge(&with);
    let f = host(vec![inspect(dir.path()), ok(""), ok(""), unauthorized()]);
    let out = carry_out_on(&Action::EdgeSiteCall, &f, "linux")
        .await
        .unwrap();
    assert!(out.contains("already in"), "{out}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join(SITE_FILE)).unwrap(),
        with
    );
    assert_eq!(backups(dir.path()), Vec::<String>::new());
}

#[tokio::test]
async fn a_failed_validate_puts_the_old_file_back_and_never_reloads() {
    let dir = edge(SITES);
    let f = host(vec![inspect(dir.path()), failed("Error: adapting config")]);
    let why = carry_out_on(&Action::EdgeSiteCall, &f, "linux")
        .await
        .unwrap_err();
    assert!(why.contains("caddy validate: failed"), "{why}");
    assert!(why.contains("is back from inorbit.hr.caddy.bak-"), "{why}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join(SITE_FILE)).unwrap(),
        SITES
    );
    assert_eq!(argvs(&f).len(), 2);
}

#[tokio::test]
async fn a_failed_reload_puts_the_old_file_back() {
    let dir = edge(SITES);
    let f = host(vec![inspect(dir.path()), ok(""), failed("reload refused")]);
    let why = carry_out_on(&Action::EdgeSiteCall, &f, "linux")
        .await
        .unwrap_err();
    assert!(why.contains("caddy reload: failed"), "{why}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join(SITE_FILE)).unwrap(),
        SITES
    );
}

#[tokio::test]
async fn a_site_that_never_answers_401_fails_after_every_try() {
    let dir = edge(SITES);
    let mut steps = vec![inspect(dir.path()), ok(""), ok("")];
    steps.extend((0..TRIES).map(|_| failed("  HTTP/1.1 525 <none>")));
    let f = host(steps);
    let why = carry_out_on(&Action::EdgeSiteCall, &f, "linux")
        .await
        .unwrap_err();
    assert!(why.contains("did not answer 401 in 12 tries"), "{why}");
    assert!(why.contains("validated and reloaded, but"), "{why}");
    assert_eq!(argvs(&f).len(), 3 + TRIES);
}

#[tokio::test]
async fn it_refuses_before_touching_anything() {
    // Not running.
    let dir = edge(SITES);
    let f = host(vec![ok(&format!("false\t{}", dir.path().display()))]);
    let why = carry_out_on(&Action::EdgeSiteCall, &f, "linux")
        .await
        .unwrap_err();
    assert!(why.contains("not running"), "{why}");
    // No (api) snippet.
    std::fs::write(dir.path().join("Caddyfile"), "(site) {\n}\n").unwrap();
    let f = host(vec![inspect(dir.path())]);
    let why = carry_out_on(&Action::EdgeSiteCall, &f, "linux")
        .await
        .unwrap_err();
    assert!(why.contains("no (api) snippet"), "{why}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join(SITE_FILE)).unwrap(),
        SITES
    );
    assert_eq!(backups(dir.path()), Vec::<String>::new());
    // Not Linux: nothing runs.
    let f = host(vec![]);
    let why = carry_out_on(&Action::EdgeSiteCall, &f, "macos")
        .await
        .unwrap_err();
    assert!(why.contains("Linux only"), "{why}");
    assert_eq!(argvs(&f), Vec::<String>::new());
}
