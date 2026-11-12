//! The service binary: reads HOST and PORT, serves, and stops cleanly on SIGINT or SIGTERM.

use std::net::TcpListener;
use std::process::ExitCode;
use std::sync::Arc;

use @@MODULE@@::obs::{Logger, Val};
use @@MODULE@@::{Service, serve, signal};

fn main() -> ExitCode {
    let log = Logger::new(std::io::stdout());
    let host = std::env::var("HOST")
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "127.0.0.1".into());
    let port = match std::env::var("PORT").ok().filter(|p| !p.is_empty()) {
        None => 8080,
        Some(p) => match p.parse::<u16>() {
            Ok(p) => p,
            Err(_) => {
                log.error(
                    "bad configuration",
                    &[("error", Val::Str("PORT must be a number from 0 to 65535"))],
                );
                return ExitCode::from(2);
            }
        },
    };
    let listener = match TcpListener::bind((host.as_str(), port)) {
        Ok(l) => l,
        Err(e) => {
            log.error("cannot listen", &[("error", Val::Str(&e.to_string()))]);
            return ExitCode::FAILURE;
        }
    };
    signal::install();
    let svc = Arc::new(Service::new(log));
    match serve(&listener, &svc, &signal::STOP) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            svc.log
                .error("stopped", &[("error", Val::Str(&e.to_string()))]);
            ExitCode::FAILURE
        }
    }
}
