//! `iohr ext packages` and `iohr packages`: every package the caller has, in one table.
//!
//! Three sources are merged by name: the account's listings from the API (its own,
//! public or private, and the private ones that name it: `GET /v1/extensions?mine=true`),
//! the extensions installed on this machine, and the folders under `~/.inorbit/ext-dev`
//! that `iohr ext dev` made for unpublished extensions. The folders are read from disk
//! and never sent anywhere.

use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::Env;
use crate::cli::Global;
use crate::commands::ext_catalogue::{clean, ident, scoped};
use crate::commands::{pages, str_of};
use crate::context::session;
use crate::error::Error;
use crate::ext::install::Store;
use crate::ext::manifest::check_name;
use crate::output::Out;

/// The most listings read, so an answer that never ends cannot hold `iohr` in a loop.
const MAX_LISTINGS: usize = 10_000;

/// The most local folders listed.
const MAX_LOCAL: usize = 1_000;

/// One package, wherever it comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Package {
    pub(crate) name: String,
    pub(crate) kind: String,
    /// `account` (the account publishes it), `shared` (a private listing names the
    /// account), `local` (a folder under `ext-dev`, unpublished) or `installed`
    /// (installed here, in none of the account's listings).
    pub(crate) source: String,
    /// `public`, `private`, `local` or `-` when not known.
    pub(crate) visibility: String,
    pub(crate) version: String,
    /// The installed version, or none.
    pub(crate) installed: Option<String>,
}

/// The names of the folders under `dir` that are extension names, sorted, at most
/// [`MAX_LOCAL`]. A missing folder is none; a link or a file is not one.
pub(crate) fn local_names(dir: &Path) -> Vec<String> {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                Out::note(&format!(
                    "Could not read {}: {e}; its local packages are not listed.",
                    dir.display()
                ));
            }
            return Vec::new();
        }
    };
    let mut names: Vec<String> = read
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| check_name(n).is_ok())
        .collect();
    names.sort();
    names.truncate(MAX_LOCAL);
    names
}

/// Merges the three sources. `installed` is `(name, version)`.
pub(crate) fn merge(
    server: &[Value],
    installed: &[(String, String)],
    local: &[String],
) -> Vec<Package> {
    let version_of = |name: &str| {
        installed
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    };
    let mut out = Vec::new();
    let mut claimed: Vec<&str> = Vec::new();
    for x in server {
        let name = ident(x);
        let short = str_of(x, "name");
        if installed.iter().any(|(n, _)| n == short) {
            claimed.push(short);
        }
        let latest = x
            .get("latest")
            .map(|v| clean(str_of(v, "version")))
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "-".to_owned());
        out.push(Package {
            kind: clean(str_of(x, "kind")),
            source: if str_of(x, "relation") == "shared" {
                "shared"
            } else {
                "account"
            }
            .to_owned(),
            visibility: clean(str_of(x, "visibility")),
            version: latest,
            installed: version_of(short),
            name,
        });
    }
    for n in local {
        out.push(Package {
            name: n.clone(),
            kind: "-".into(),
            source: "local".into(),
            visibility: "local".into(),
            version: "-".into(),
            // A published package of the same name already shows the installation.
            installed: version_of(n).filter(|_| !claimed.contains(&n.as_str())),
        });
    }
    let mut rest: Vec<&(String, String)> = installed
        .iter()
        .filter(|(n, _)| !claimed.contains(&n.as_str()) && !local.contains(n))
        .collect();
    rest.sort();
    for (n, v) in rest {
        out.push(Package {
            name: n.clone(),
            kind: "-".into(),
            source: "installed".into(),
            visibility: "-".into(),
            version: v.clone(),
            installed: Some(v.clone()),
        });
    }
    out
}

fn print(packages: &[Package], out: Out) {
    if out.json {
        Out::print_json(&packages);
    } else if packages.is_empty() {
        Out::note("No packages: none published, none installed, none in progress here.");
    } else {
        let rows: Vec<Vec<String>> = packages
            .iter()
            .map(|p| {
                vec![
                    p.name.clone(),
                    p.kind.clone(),
                    p.source.clone(),
                    p.visibility.clone(),
                    p.version.clone(),
                    p.installed.clone().unwrap_or_else(|| "no".into()),
                ]
            })
            .collect();
        Out::table(
            &[
                "NAME",
                "KIND",
                "SOURCE",
                "VISIBILITY",
                "VERSION",
                "INSTALLED",
            ],
            &rows,
        );
    }
}

/// `iohr ext packages`. When the API cannot be read (not signed in, no scope), the
/// installed and local packages are still listed, and the failure is returned.
pub(crate) async fn packages(g: &Global, env: &Env, store: &Store, out: Out) -> Result<(), Error> {
    let mut installed = Vec::new();
    for name in store.lock()?.entries.keys() {
        if let Some((entry, _, _)) = store.installed(name)? {
            installed.push((entry.name, entry.version));
        }
    }
    let local = etcetera::home_dir()
        .map(|h| local_names(&h.join(".inorbit").join("ext-dev")))
        .unwrap_or_default();
    let fetched = async {
        let s = session(g, env).await?;
        pages(
            &s.api,
            "/v1/extensions",
            &[("mine", "true")],
            "extensions",
            MAX_LISTINGS,
        )
        .await
        .map_err(scoped)
    }
    .await;
    match fetched {
        Ok(server) => {
            print(&merge(&server, &installed, &local), out);
            if server.len() >= MAX_LISTINGS {
                Out::note("Stopped at 10000 listings.");
            }
            Ok(())
        }
        Err(e) => {
            Out::note(
                "Could not read the account's published packages; showing this machine's only.",
            );
            print(&merge(&[], &installed, &local), out);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn listing(publisher: &str, name: &str, vis: &str, relation: &str) -> Value {
        json!({"publisher": publisher, "name": name, "kind": "program", "visibility": vis,
               "relation": relation, "latest": {"version": "1.2.0"}})
    }

    #[test]
    fn merges_server_installed_and_local_by_name() {
        let server = [
            listing("acme", "billing", "private", "own"),
            listing("acme", "shared-one", "private", "shared"),
            listing("acme", "open", "public", "own"),
        ];
        let installed = vec![
            ("billing".to_owned(), "1.0.0".to_owned()),
            ("agent".to_owned(), "0.9.0".to_owned()),
            ("wip".to_owned(), "0.1.0".to_owned()),
        ];
        let local = vec!["wip".to_owned(), "sketch".to_owned()];
        let p = merge(&server, &installed, &local);
        let row = |n: &str| p.iter().find(|x| x.name == n).unwrap();
        assert_eq!(row("acme/billing").source, "account");
        assert_eq!(row("acme/billing").visibility, "private");
        assert_eq!(row("acme/billing").installed.as_deref(), Some("1.0.0"));
        assert_eq!(row("acme/shared-one").source, "shared");
        assert_eq!(row("acme/shared-one").installed, None);
        assert_eq!(row("acme/open").visibility, "public");
        assert_eq!(row("sketch").source, "local");
        assert_eq!(row("wip").installed.as_deref(), Some("0.1.0"));
        // A local folder named like a published, installed package does not report the
        // same installation twice.
        let p2 = merge(
            &[listing("acme", "billing", "private", "own")],
            &[("billing".to_owned(), "1.0.0".to_owned())],
            &["billing".to_owned()],
        );
        assert_eq!(p2.iter().filter(|x| x.installed.is_some()).count(), 1);
        assert_eq!(row("agent").source, "installed");
        // wip is listed once, as local; billing once, as the listing.
        assert_eq!(p.iter().filter(|x| x.name == "wip").count(), 1);
        assert!(p.iter().all(|x| x.name != "billing"));
        assert_eq!(p.len(), 6);
    }

    #[test]
    fn local_names_skip_files_links_and_bad_names() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("good")).unwrap();
        std::fs::create_dir(d.path().join("Bad Name")).unwrap();
        std::fs::write(d.path().join("afile"), b"x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(d.path(), d.path().join("link")).unwrap();
        assert_eq!(local_names(d.path()), ["good"]);
        assert_eq!(local_names(&d.path().join("missing")).len(), 0);
    }
}
