#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests fail by panicking"
)]

use std::path::Path;
use std::time::{Duration, Instant};

use semver::Version;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

// What each upstream answered on 2026-11-06, cut to the first entries (the fields kept are the
// ones the real responses carry).
const RUST: &str = include_str!("../../tests/data/toolchain/rust.toml");
const GO: &str = include_str!("../../tests/data/toolchain/go.json");
const NODE: &str = include_str!("../../tests/data/toolchain/node.json");
const PYTHON: &str = include_str!("../../tests/data/toolchain/python.json");

fn v(s: &str) -> Version {
    s.parse().unwrap()
}

#[test]
fn rust_reads_the_version_under_pkg_rust_and_not_cargos() {
    assert_eq!(parse_rust(RUST), Some(v("1.99.1")));
    assert_eq!(parse_rust("[pkg.cargo]\nversion = \"0.100.0 (x)\"\n"), None);
    assert_eq!(
        parse_rust("[pkg.rust]\n[pkg.rust.target.x]\nversion = \"9.9.9\"\n"),
        None
    );
    assert_eq!(parse_rust("[pkg.rust]\nversion = 1.99\n"), None);
    assert_eq!(parse_rust(""), None);
}

#[test]
fn go_reads_the_first_stable_release() {
    assert_eq!(parse_go(GO), Some(v("1.27.3")));
    let skip = r#"[{"version":"go1.28rc1","stable":false},{"version":"go1.27.3","stable":true}]"#;
    assert_eq!(parse_go(skip), Some(v("1.27.3")));
    assert_eq!(
        parse_go(r#"[{"version":"go1.28rc1","stable":false}]"#),
        None
    );
    assert_eq!(parse_go("not json"), None);
    assert_eq!(parse_go(r#"[{"version":"1.27.3","stable":true}]"#), None);
}

#[test]
fn node_reads_the_newest_lts_not_the_newest_release() {
    // The fixture's first entry is a v27 alpha, the second the newest LTS line (`Lithium`).
    assert_eq!(parse_node(NODE), Some(v("26.13.0")));
    assert_eq!(
        parse_node(r#"[{"version":"v27.0.0","lts":false}]"#),
        None,
        "no LTS in the list"
    );
    assert_eq!(parse_node("{}"), None);
}

#[test]
fn python_reads_the_highest_stable_three_not_a_release_candidate_or_the_manager() {
    assert_eq!(parse_python(PYTHON), Some(v("3.15.2")));
    let only_rc = r#"[{"name":"Python 3.16.0rc1","is_published":true,"pre_release":false}]"#;
    assert_eq!(parse_python(only_rc), None);
    let unpublished = r#"[{"name":"Python 3.16.0","is_published":false,"pre_release":false}]"#;
    assert_eq!(parse_python(unpublished), None);
    assert_eq!(parse_python("[]"), None);
}

#[test]
fn each_language_is_found_by_its_version_program() {
    let c = |w: &[&str]| w.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        Lang::of_toolchain(&c(&["rustc", "--version"])),
        Some(Lang::Rust)
    );
    assert_eq!(Lang::of_toolchain(&c(&["go", "version"])), Some(Lang::Go));
    assert_eq!(
        Lang::of_toolchain(&c(&["node", "--version"])),
        Some(Lang::Node)
    );
    assert_eq!(
        Lang::of_toolchain(&c(&["python3", "--version"])),
        Some(Lang::Python)
    );
    assert_eq!(
        Lang::of_toolchain(&c(&["/usr/bin/python", "-V"])),
        Some(Lang::Python)
    );
    assert_eq!(Lang::of_toolchain(&c(&["java", "-version"])), None);
    assert_eq!(Lang::of_toolchain(&[]), None);
}

#[test]
fn behind_equal_and_ahead() {
    let j = |i: &str, l: &str| judge(Some(v(i)), Ok(v(l)));
    assert!(matches!(j("1.90.0", "1.93.0"), State::Behind { .. }));
    assert!(matches!(j("1.93.0", "1.93.0"), State::Current { .. }));
    assert!(matches!(j("1.99.1", "1.93.0"), State::Ahead { .. }));
    // A patch behind is behind; a two-digit minor is not read as text.
    assert!(matches!(j("1.99.0", "1.99.1"), State::Behind { .. }));
    assert!(matches!(j("1.9.0", "1.10.0"), State::Behind { .. }));
}

#[test]
fn a_lookup_that_failed_never_shows_a_latest_version() {
    let s = judge(Some(v("1.90.0")), Err("Rust did not answer".into()));
    assert_eq!(
        s,
        State::Unchecked {
            installed: Some(v("1.90.0"))
        }
    );
    assert_eq!(
        s.hint(Lang::Rust),
        "rustc 1.90.0, could not check the latest"
    );
    assert_eq!(
        s.said(Lang::Rust),
        "could not check whether Rust 1.90.0 is the latest stable"
    );
    let none = judge(None, Ok(v("1.93.0")));
    assert_eq!(none, State::Unchecked { installed: None });
    assert_eq!(none.hint(Lang::Go), "could not check the latest");
    assert!(!none.said(Lang::Go).contains("1.93"));
}

#[test]
fn the_hint_names_the_tool_and_what_is_out() {
    let behind = judge(Some(v("1.90.0")), Ok(v("1.93.0")));
    assert_eq!(behind.hint(Lang::Rust), "rustc 1.90.0, 1.93.0 is out");
    assert_eq!(
        behind.said(Lang::Rust),
        "Rust 1.90.0 is behind the latest stable, 1.93.0"
    );
    let ahead = judge(Some(v("1.101.0")), Ok(v("1.99.1")));
    assert_eq!(
        ahead.hint(Lang::Rust),
        "rustc 1.101.0, newer than stable 1.99.1"
    );
    let same = judge(Some(v("3.15.2")), Ok(v("3.15.2")));
    assert_eq!(same.hint(Lang::Python), "python 3.15.2, up to date");
}

async fn serve(lang_path: &str, body: &str) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(lang_path))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&s)
        .await;
    s
}

#[tokio::test]
async fn the_lookup_reads_each_upstream_from_a_local_server() {
    for (lang, p, body, want) in [
        (Lang::Rust, "/rust", RUST, "1.99.1"),
        (Lang::Go, "/go", GO, "1.27.3"),
        (Lang::Node, "/node", NODE, "26.13.0"),
        (Lang::Python, "/python", PYTHON, "3.15.2"),
    ] {
        let s = serve(p, body).await;
        let got = Upstream::at(&s.uri()).latest(lang).await;
        assert_eq!(got, Ok(v(want)), "{p}");
    }
}

#[tokio::test]
async fn a_server_error_or_a_body_that_is_not_the_format_is_an_error_line() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/go"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/rust"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>captive portal</html>"))
        .mount(&s)
        .await;
    let up = Upstream::at(&s.uri());
    assert_eq!(up.latest(Lang::Go).await, Err("Go did not answer".into()));
    assert_eq!(
        up.latest(Lang::Rust).await,
        Err("Rust's answer had no version".into())
    );
}

/// `probe_line` on its own thread, as the lookup runs it (it builds a runtime of its own).
async fn probed(lang: Lang, line: &'static str, up: Upstream) -> (State, Duration) {
    tokio::task::spawn_blocking(move || {
        let t = Instant::now();
        (probe_line(lang, line, &up), t.elapsed())
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn offline_is_could_not_check_and_fast() {
    // Nothing listens on port 1: the connection is refused at once.
    let (s, took) = probed(
        Lang::Rust,
        "rustc 1.90.0 (abc 2026-01-01)",
        Upstream::at("http://127.0.0.1:1"),
    )
    .await;
    assert_eq!(
        s,
        State::Unchecked {
            installed: Some(v("1.90.0"))
        }
    );
    assert!(took < Duration::from_secs(3), "{took:?}");
}

#[tokio::test]
async fn a_server_that_hangs_is_cut_off_at_the_budget() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/go"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(GO)
                .set_delay(Duration::from_secs(8)),
        )
        .mount(&s)
        .await;
    let (state, took) = probed(
        Lang::Go,
        "go version go1.26.1 darwin/arm64",
        Upstream::at(&s.uri()),
    )
    .await;
    assert_eq!(
        state,
        State::Unchecked {
            installed: Some(v("1.26.1"))
        }
    );
    assert!(
        took >= Duration::from_secs(2) && took < Duration::from_secs(5),
        "{took:?}"
    );
}

#[tokio::test]
async fn a_version_line_with_no_version_is_unchecked_without_a_request() {
    let (s, took) = probed(
        Lang::Node,
        "command not found",
        Upstream::at("http://127.0.0.1:1"),
    )
    .await;
    assert_eq!(s, State::Unchecked { installed: None });
    assert!(took < Duration::from_secs(1));
}

#[tokio::test]
async fn behind_is_found_end_to_end_with_a_real_body() {
    let s = serve("/python", PYTHON).await;
    let (state, _) = probed(Lang::Python, "Python 3.14.7", Upstream::at(&s.uri())).await;
    assert_eq!(
        state,
        State::Behind {
            installed: v("3.14.7"),
            latest: v("3.15.2")
        }
    );
}

#[test]
fn the_upgrade_follows_what_installed_the_toolchain() {
    let latest = v("1.99.1");
    let all = |_: &str| true;
    let none = |_: &str| false;
    let up = |lang, bin: &str, has: &dyn Fn(&str) -> bool| {
        upgrade_for(lang, Some(Path::new(bin)), &latest, has)
    };
    let run = |u: &Upgrade| u.run.clone().unwrap();

    let u = up(Lang::Go, "/opt/homebrew/bin/go", &all);
    assert_eq!(u.shown, "brew upgrade go");
    assert_eq!(run(&u), ["brew", "upgrade", "go"]);
    assert_eq!(
        up(Lang::Node, "/usr/local/Cellar/node/26.1/bin/node", &all).shown,
        "brew upgrade node"
    );
    assert_eq!(
        up(Lang::Rust, "/home/u/.cargo/bin/rustc", &all).shown,
        "rustup update stable"
    );
    // A brew Rust has no rustup, even when rustup is also installed.
    assert_eq!(
        up(Lang::Rust, "/opt/homebrew/bin/rustc", &all).shown,
        "brew upgrade rust"
    );
    assert_eq!(
        up(Lang::Node, "/home/u/.local/share/mise/shims/node", &all).shown,
        "mise upgrade node"
    );
    // asdf, pyenv and nvm are shown, never run.
    let a = up(Lang::Go, "/home/u/.asdf/shims/go", &none);
    assert_eq!(a.shown, "asdf install golang latest");
    assert!(a.run.is_none());
    let p = upgrade_for(
        Lang::Python,
        Some(Path::new("/home/u/.pyenv/shims/python3")),
        &v("3.15.2"),
        none,
    );
    assert_eq!(p.shown, "pyenv install 3.15.2");
    assert!(p.run.is_none());
    assert!(
        up(Lang::Node, "/home/u/.nvm/versions/node/v26/bin/node", &none)
            .run
            .is_none()
    );
}

#[test]
fn an_unknown_install_gets_the_download_page_and_nothing_to_run() {
    let u = upgrade_for(
        Lang::Go,
        Some(Path::new("/usr/local/go/bin/go")),
        &v("1.27.3"),
        |_| false,
    );
    assert_eq!(u.shown, "download 1.27.3 from https://go.dev/dl/");
    assert!(u.run.is_none());
    assert!(run_upgrade(&u).is_err());
    // Homebrew paths without brew on PATH are not a brew command.
    let u = upgrade_for(
        Lang::Go,
        Some(Path::new("/opt/homebrew/bin/go")),
        &v("1.27.3"),
        |_| false,
    );
    assert!(u.run.is_none());
    let u = upgrade_for(Lang::Python, None, &v("3.15.2"), |_| true);
    assert_eq!(
        u.shown,
        "download 3.15.2 from https://www.python.org/downloads/"
    );
}
