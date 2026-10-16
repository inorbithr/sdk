//! `iohr tunnel`: a loopback pipe from this machine into an environment's Envoy edge.
//!
//! The platform's hosts (`www`, `console`, `docs`, `auth`) all answer on Envoy's edge port
//! on the machine that runs the environment, bound to 127.0.0.1. `*.localhost` resolves to
//! loopback by itself (RFC 6761 section 6.3), so forwarding that one port opens the real
//! site at `http://www.localhost:<port>/` with no DNS, hosts file or certificate. The
//! forward ends at Envoy: a service is never reached directly, so every gate stays.
//!
//! Core's `devops/ci/envs.toml` owns the ports; `environments.toml` mirrors them (`mise run env:drift` compares), and
//! `mise run local:tunnel` passes core's values with `--edge-port` and `--engine-port`.

use std::fmt::Write as _;

use crate::cli::{Global, Tunnel};
use crate::error::Error;
use crate::output::Out;

/// The hosts the edge serves, in the order printed.
const HOSTS: [&str; 4] = ["www", "console", "docs", "auth"];

/// What a tunnel does: the ssh arguments and the addresses it opens.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) ssh_args: Vec<String>,
    pub(crate) urls: Vec<(String, String)>,
    pub(crate) listen: u16,
}

pub(crate) fn plan(g: &Global, args: &Tunnel) -> Result<Plan, Error> {
    if args.host.trim().is_empty() || args.host.starts_with('-') {
        return Err(Error::Usage(
            "--host names an ssh target (or set IOHR_TUNNEL_HOST)".into(),
        ));
    }
    let env = g.environment();
    let remote = args.edge_port.or(env.edge_port).ok_or_else(|| {
        Error::Usage("this environment has no edge port; pass --edge-port".into())
    })?;
    let listen = args.port.unwrap_or(remote);
    let mut ssh_args: Vec<String> = [
        "-N",
        "-o",
        "ExitOnForwardFailure=yes",
        "-o",
        "ServerAliveInterval=30",
        "-o",
        "ServerAliveCountMax=3",
        "-L",
    ]
    .map(String::from)
    .to_vec();
    ssh_args.push(format!("127.0.0.1:{listen}:127.0.0.1:{remote}"));
    let engine = args
        .engine_port
        .or(env.engine_port)
        .filter(|_| !args.no_engine);
    if let Some(engine) = engine {
        ssh_args.push("-L".into());
        ssh_args.push(format!("127.0.0.1:{engine}:127.0.0.1:{engine}"));
    }
    ssh_args.push("--".into());
    ssh_args.push(args.host.clone());
    let urls = HOSTS
        .iter()
        .map(|h| ((*h).to_owned(), format!("http://{h}.localhost:{listen}/")))
        .collect();
    Ok(Plan {
        ssh_args,
        urls,
        listen,
    })
}

pub(crate) fn run(g: &Global, args: &Tunnel, out: Out) -> Result<(), Error> {
    let plan = plan(g, args)?;
    if out.json {
        Out::print_json(&serde_json::json!({
            "listen": format!("127.0.0.1:{}", plan.listen),
            "ssh": plan.ssh_args,
            "urls": plan.urls.iter().cloned().collect::<std::collections::BTreeMap<_, _>>(),
        }));
    } else {
        let mut text = format!("Listening on 127.0.0.1:{} (Ctrl-C to stop)\n", plan.listen);
        for (name, url) in &plan.urls {
            let _ = writeln!(text, "  {name:<8} {url}");
        }
        Out::raw(text.as_bytes());
    }
    if args.dry_run {
        return Ok(());
    }
    start(&plan)
}

#[cfg(unix)]
fn start(plan: &Plan) -> Result<(), Error> {
    use std::os::unix::process::CommandExt;
    // exec replaces this process, so Ctrl-C and the exit code are ssh's own.
    let err = std::process::Command::new("ssh")
        .args(&plan.ssh_args)
        .exec();
    Err(Error::Failed(format!("could not start ssh: {err}")))
}

#[cfg(not(unix))]
fn start(plan: &Plan) -> Result<(), Error> {
    let status = std::process::Command::new("ssh")
        .args(&plan.ssh_args)
        .status()
        .map_err(|e| Error::Failed(format!("could not start ssh: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Child(
            status.code().map_or(1, |c| c.clamp(1, 255) as u8),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::plan;
    use crate::cli::{Cli, Command};
    use clap::Parser;

    fn tunnel(argv: &[&str]) -> Result<super::Plan, crate::error::Error> {
        let mut full = vec!["iohr", "tunnel"];
        full.extend_from_slice(argv);
        let cli = Cli::try_parse_from(full).expect("parses");
        match cli.command {
            Command::Tunnel(t) => plan(&cli.global, &t),
            _ => panic!("not tunnel"),
        }
    }

    #[test]
    fn production_is_the_default_and_the_site_opens_on_its_edge_port() {
        let p = tunnel(&["--host", "h"]).unwrap();
        assert_eq!(p.listen, 18080);
        assert_eq!(
            p.urls[0],
            ("www".into(), "http://www.localhost:18080/".into())
        );
        assert!(
            p.ssh_args
                .contains(&"127.0.0.1:18080:127.0.0.1:18080".to_owned())
        );
        assert_eq!(p.ssh_args.last().unwrap(), "h");
    }

    #[test]
    fn local_port_override_moves_only_the_listening_side() {
        let p = tunnel(&[
            "--host",
            "h",
            "--env",
            "local",
            "--no-engine",
            "--port",
            "9000",
        ])
        .unwrap();
        assert!(
            p.ssh_args
                .contains(&"127.0.0.1:9000:127.0.0.1:28080".to_owned())
        );
        assert_eq!(p.urls[1].1, "http://console.localhost:9000/");
    }

    #[test]
    fn core_can_hand_over_its_own_ports_and_the_engine_lb() {
        let p = tunnel(&[
            "--host",
            "h",
            "--edge-port",
            "30000",
            "--engine-port",
            "30001",
        ])
        .unwrap();
        assert!(
            p.ssh_args
                .contains(&"127.0.0.1:30000:127.0.0.1:30000".to_owned())
        );
        assert!(
            p.ssh_args
                .contains(&"127.0.0.1:30001:127.0.0.1:30001".to_owned())
        );
    }

    #[test]
    fn local_forwards_the_engine_lb_unless_told_not_to() {
        let p = tunnel(&["--env", "local", "--host", "h"]).unwrap();
        assert!(
            p.ssh_args
                .contains(&"127.0.0.1:25051:127.0.0.1:25051".to_owned())
        );
        let p = tunnel(&["--env", "local", "--host", "h", "--no-engine"]).unwrap();
        assert_eq!(p.ssh_args.iter().filter(|a| *a == "-L").count(), 1);
        let p = tunnel(&["--host", "h"]).unwrap();
        assert_eq!(p.ssh_args.iter().filter(|a| *a == "-L").count(), 1);
    }

    #[test]
    fn every_forward_binds_loopback_and_ssh_cannot_take_the_host_as_an_option() {
        let p = tunnel(&["--host", "h", "--engine-port", "1"]).unwrap();
        for w in p.ssh_args.windows(2).filter(|w| w[0] == "-L") {
            assert!(w[1].starts_with("127.0.0.1:") && w[1].contains(":127.0.0.1:"));
        }
        assert!(tunnel(&["--host=-oProxyCommand=x"]).is_err());
        assert!(tunnel(&["--host", " "]).is_err());
    }

    #[test]
    fn an_unknown_environment_is_refused() {
        assert!(
            Cli::try_parse_from(["iohr", "tunnel", "--host", "h", "--env", "staging"]).is_err()
        );
    }
}
