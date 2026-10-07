//! `iohr ext search`, `iohr ext show` and the `publisher/name` form of `iohr ext
//! install`: the platform's extensions catalogue (RFC 0073 phase 1), read over the API
//! host. The catalogue only says what to install; the artifact still comes from the
//! registry and passes every check `install` makes (ADR 0012), and the catalogue's
//! signer must be the signer those checks found.

use serde_json::Value;

use crate::Env;
use crate::cli::Global;
use crate::commands::{day, pages, str_of};
use crate::context::{Session, session};
use crate::error::Error;
use crate::ext::install::Fetched;
use crate::ext::manifest::{check_name, check_version};
use crate::ext::oci::Digest;
use crate::output::Out;

/// The publisher a bare name means (RFC 0073: `agent` is `inorbit/agent`).
pub(crate) const FIRST_PARTY: &str = "inorbit";

/// The most versions `show` and `install` read (four pages of the catalogue's largest).
/// The catalogue answers newest first, so the newest release is always among them; a
/// version older than the 800 newest is not found by `@VERSION` or `@sha256:`.
const MAX_VERSIONS: usize = 800;

/// The most listings `search --all` reads, so an answer that never ends cannot hold
/// `iohr` in a loop.
const MAX_SEARCH: usize = 10_000;

/// Catalogue text with control, bidirectional and invisible format characters taken
/// out, so a listing cannot move the cursor, recolour the terminal or reorder a line.
fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(
                    c,
                    '\u{00AD}'
                        | '\u{061C}'
                        | '\u{180E}'
                        | '\u{200B}'..='\u{200F}'
                        | '\u{202A}'..='\u{202E}'
                        | '\u{2060}'..='\u{206F}'
                        | '\u{FEFF}'
                        | '\u{FFF9}'..='\u{FFFB}'
                )
        })
        .collect()
}

/// `publisher/name` from an answer, each shown only when it is a valid identifier.
fn ident(x: &Value) -> String {
    let p = str_of(x, "publisher");
    let n = str_of(x, "name");
    format!(
        "{}/{}",
        if check_publisher(p).is_ok() { p } else { "?" },
        if check_name(n).is_ok() { n } else { "?" }
    )
}

fn list_of(v: &Value, k: &str) -> Vec<String> {
    v.get(k)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(clean).collect())
        .unwrap_or_default()
}

fn flag(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

/// Checks a publisher's namespace: 2 to 39 lower-case letters, digits and `-`.
fn check_publisher(p: &str) -> Result<(), Error> {
    let ok = (2..=39).contains(&p.len())
        && p.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !p.starts_with('-')
        && !p.ends_with('-');
    if ok {
        Ok(())
    } else {
        Err(Error::Usage(
            "a publisher has 2 to 39 lower-case letters, digits and '-'".into(),
        ))
    }
}

/// `publisher/name`, both checked.
pub(crate) fn parse_listing(s: &str) -> Result<(String, String), Error> {
    let Some((publisher, name)) = s.split_once('/') else {
        return Err(Error::Usage(format!(
            "`{}` is not PUBLISHER/NAME, such as inorbit/agent",
            clean(s)
        )));
    };
    check_publisher(publisher)?;
    check_name(name)?;
    Ok((publisher.to_owned(), name.to_owned()))
}

fn scoped(e: inorbithr::Error) -> Error {
    match e.status() {
        Some(403) => Error::with_hint(
            e,
            "Reading the extensions catalogue needs the extensions:read scope, or a person \
             signed in with `iohr login`.",
        ),
        _ => e.into(),
    }
}

/// One listing, by `publisher/name`. A private listing this account may not see is
/// `NOT_FOUND`, the same as one that does not exist.
async fn listing(s: &Session, publisher: &str, name: &str) -> Result<Value, Error> {
    let v: Value = s
        .api
        .get(&format!("/v1/extensions/{publisher}/{name}"), &[])
        .await
        .map_err(|e| {
            if e.status() == Some(404) {
                Error::with_hint(
                    e,
                    "There is no such extension in the catalogue, or not one this account \
                     may see; `iohr ext search` lists what it can.",
                )
            } else {
                scoped(e)
            }
        })?;
    Ok(v.get("extension").cloned().unwrap_or(Value::Null))
}

async fn versions(s: &Session, publisher: &str, name: &str) -> Result<Vec<Value>, Error> {
    pages(
        &s.api,
        &format!("/v1/extensions/{publisher}/{name}/versions"),
        &[],
        "versions",
        MAX_VERSIONS,
    )
    .await
    .map_err(scoped)
}

/// `iohr ext search`.
pub(crate) async fn search(
    g: &Global,
    env: &Env,
    query: Option<&str>,
    kind: Option<&str>,
    all: bool,
    page_size: Option<u32>,
    out: Out,
) -> Result<(), Error> {
    if let Some(k) = kind
        && (k.is_empty() || k.len() > 32 || !k.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'))
    {
        return Err(Error::Usage(
            "a kind is a word such as program, agent-plugin, web, connector or assistant-plugin"
                .into(),
        ));
    }
    let s = session(g, env).await?;
    let mut q: Vec<(&str, &str)> = Vec::new();
    if let Some(text) = query.filter(|t| !t.is_empty()) {
        q.push(("query", text));
    }
    if let Some(k) = kind {
        q.push(("kind", k));
    }
    let (found, more) = if all {
        let found = pages(&s.api, "/v1/extensions", &q, "extensions", MAX_SEARCH)
            .await
            .map_err(scoped)?;
        let capped = found.len() >= MAX_SEARCH;
        (found, capped)
    } else {
        let size = page_size.map(|n| n.to_string());
        if let Some(n) = &size {
            q.push(("page_size", n));
        }
        let page: Value = s.api.get("/v1/extensions", &q).await.map_err(scoped)?;
        let found = page
            .get("extensions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        (found, !str_of(&page, "next_page_token").is_empty())
    };
    if out.json {
        Out::print_json(&found);
    } else if found.is_empty() {
        Out::note("No extensions match.");
    } else {
        let rows: Vec<Vec<String>> = found.iter().map(search_row).collect();
        Out::table(
            &["NAME", "KIND", "PUBLISHER", "LATEST", "VISIBILITY"],
            &rows,
        );
    }
    if more {
        Out::note(if all {
            "Stopped at 10000 listings: narrow the search with a query or --kind."
        } else {
            "More match: --all lists every one, --page-size N more at a time."
        });
    }
    Ok(())
}

fn search_row(x: &Value) -> Vec<String> {
    let latest = if flag(x, "coming") {
        "coming".to_owned()
    } else {
        x.get("latest")
            .map(|v| clean(str_of(v, "version")))
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "-".to_owned())
    };
    vec![
        ident(x),
        clean(str_of(x, "kind")),
        publisher_words(x),
        latest,
        clean(str_of(x, "visibility")),
    ]
}

/// The publisher as a person reads it: its display name, and `first party` for InOrbit.
/// The display name is the publisher's own words, so the namespace is always shown
/// beside it: a display name cannot pass for another publisher or for InOrbit.
fn publisher_words(x: &Value) -> String {
    let ns = str_of(x, "publisher");
    let ns = if check_publisher(ns).is_ok() { ns } else { "?" };
    let shown = clean(str_of(x, "publisher_name"));
    let party = if flag(x, "first_party") {
        ", first party"
    } else {
        ""
    };
    if shown.is_empty() {
        format!(
            "{ns}{}",
            if party.is_empty() {
                ""
            } else {
                " (first party)"
            }
        )
    } else {
        format!("{shown} ({ns}{party})")
    }
}

/// `iohr ext show PUBLISHER/NAME`.
pub(crate) async fn show(g: &Global, env: &Env, listing_ref: &str, out: Out) -> Result<(), Error> {
    let (publisher, name) = parse_listing(listing_ref)?;
    let s = session(g, env).await?;
    let ext = listing(&s, &publisher, &name).await?;
    let vs = versions(&s, &publisher, &name).await?;
    // The publisher's proved domain; a publisher the account sees no listing of is
    // NOT_FOUND, which leaves the domain out rather than failing `show`.
    let publ: Option<Value> = match s
        .api
        .get::<Value>(&format!("/v1/extensions/{publisher}"), &[])
        .await
    {
        Ok(v) => v.get("publisher").cloned(),
        Err(e) if e.status() == Some(404) => None,
        Err(e) => return Err(scoped(e)),
    };
    let install = format!("iohr ext install {publisher}/{name}");
    if out.json {
        Out::print_json(&serde_json::json!({
            "extension": ext, "publisher": publ, "versions": vs, "install": install,
        }));
        return Ok(());
    }
    print_show(&ext, publ.as_ref(), &vs, &install);
    Ok(())
}

fn evidence_words(v: &Value) -> String {
    let Some(e) = v.get("evidence") else {
        return "no evidence yet".into();
    };
    let ids = list_of(e, "record_ids");
    let summary = clean(str_of(e, "summary"));
    match (summary.is_empty(), ids.is_empty()) {
        (true, true) => "no evidence yet".into(),
        (false, true) => summary,
        (true, false) => format!("records {}", ids.join(", ")),
        (false, false) => format!("{summary} (records {})", ids.join(", ")),
    }
}

fn print_show(ext: &Value, publ: Option<&Value>, vs: &[Value], install: &str) {
    let latest = ext.get("latest");
    let mut who = publisher_words(ext);
    if let Some(p) = publ {
        let domain = clean(str_of(p, "domain"));
        if !domain.is_empty() {
            who = format!("{who}, {domain}");
        }
    }
    let scopes = latest.map(|v| list_of(v, "scopes")).unwrap_or_default();
    let privileges = latest.map(|v| list_of(v, "privileges")).unwrap_or_default();
    let signer = latest
        .map(|v| clean(str_of(v, "signer")))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| clean(str_of(ext, "signer")));
    let mut pairs = vec![
        ("name", ident(ext)),
        ("kind", clean(str_of(ext, "kind"))),
        ("description", clean(str_of(ext, "description"))),
        ("publisher", who),
        ("visibility", clean(str_of(ext, "visibility"))),
        ("source", clean(str_of(ext, "source"))),
        (
            "signer",
            if signer.is_empty() {
                "none named".into()
            } else {
                signer
            },
        ),
        (
            "latest",
            if flag(ext, "coming") {
                "not released yet".into()
            } else {
                latest
                    .map(|v| clean(str_of(v, "version")))
                    .unwrap_or_default()
            },
        ),
        (
            "scopes",
            if scopes.is_empty() {
                "no API access".into()
            } else {
                scopes.join(", ")
            },
        ),
    ];
    if privileges.is_empty() {
        pairs.push(("privileges", "none".into()));
    }
    pairs.push((
        "evidence",
        latest.map_or_else(|| "no evidence yet".into(), evidence_words),
    ));
    Out::pairs(&pairs);
    if !privileges.is_empty() {
        let names: Vec<&str> = privileges.iter().map(String::as_str).collect();
        Out::raw(
            format!(
                "privileges   its system service holds these Linux capabilities; iohr grants none of them\n{}\n",
                super::ext::privilege_lines(&names)
            )
            .as_bytes(),
        );
    }
    if !vs.is_empty() {
        let rows: Vec<Vec<String>> = vs
            .iter()
            .map(|v| {
                vec![
                    clean(str_of(v, "version")),
                    day(&clean(str_of(v, "published_at"))),
                    Digest::parse(str_of(v, "digest"))
                        .map_or_else(|_| clean(str_of(v, "digest")), |d| d.short().to_owned()),
                    list_of(v, "platforms").join(" "),
                    evidence_words(v),
                ]
            })
            .collect();
        Out::raw(b"\n");
        Out::table(
            &["VERSION", "PUBLISHED", "DIGEST", "PLATFORMS", "EVIDENCE"],
            &rows,
        );
    }
    if flag(ext, "coming") || vs.is_empty() {
        Out::raw(b"\nNothing to install yet: no version is released.\n");
    } else {
        Out::raw(format!("\n{install}\n").as_bytes());
    }
}

/// What the catalogue says to install for `publisher/name[@version|@sha256:...]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resolved {
    pub(crate) publisher: String,
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) digest: Digest,
    /// The version's signer, as the catalogue records it.
    pub(crate) signer: String,
    /// The listing's signing identity (may end in `@refs/tags/`).
    pub(crate) listing_signer: String,
    pub(crate) scopes: Vec<String>,
    pub(crate) privileges: Vec<String>,
}

/// What `at` (after `@`) asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum At {
    Newest,
    Version(String),
    Digest(Digest),
}

pub(crate) fn parse_at(at: Option<&str>) -> Result<At, Error> {
    match at {
        None => Ok(At::Newest),
        Some(d) if d.starts_with("sha256:") => Ok(At::Digest(Digest::parse(d)?)),
        Some(v) => {
            check_version(v).map_err(|e| Error::Usage(e.to_string()))?;
            Ok(At::Version(v.to_owned()))
        }
    }
}

/// Resolves a listing through the catalogue: the version asked for, or the newest
/// release (the highest version without a pre-release, else the highest pre-release,
/// the rule the registry form uses).
pub(crate) async fn resolve(
    g: &Global,
    env: &Env,
    publisher: &str,
    name: &str,
    at: &At,
) -> Result<Resolved, Error> {
    let s = session(g, env).await?;
    let ext = listing(&s, publisher, name).await?;
    let vs = versions(&s, publisher, name).await?;
    pick(&ext, &vs, publisher, name, at)
}

/// The choice `resolve` makes, from the catalogue's answers alone.
pub(crate) fn pick(
    ext: &Value,
    vs: &[Value],
    publisher: &str,
    name: &str,
    at: &At,
) -> Result<Resolved, Error> {
    let kind = str_of(ext, "kind");
    if !kind.is_empty() && kind != "program" {
        return Err(Error::Usage(format!(
            "{publisher}/{name} is a {} extension; iohr installs programs only (RFC 0073)",
            clean(kind)
        )));
    }
    if flag(ext, "coming") || vs.is_empty() {
        return Err(Error::Failed(format!(
            "{publisher}/{name} has no released version yet"
        )));
    }
    let chosen: &Value = match at {
        At::Version(want) => vs
            .iter()
            .find(|v| str_of(v, "version") == want)
            .ok_or_else(|| {
                Error::Failed(format!(
                    "the catalogue has no version {want} of {publisher}/{name}; \
                     `iohr ext show {publisher}/{name}` lists them"
                ))
            })?,
        At::Digest(want) => vs
            .iter()
            .find(|v| str_of(v, "digest") == want.as_str())
            .ok_or_else(|| {
                Error::Failed(format!(
                    "the catalogue has no version of {publisher}/{name} at {want}"
                ))
            })?,
        At::Newest => {
            let mut known: Vec<(semver::Version, &Value)> = vs
                .iter()
                .filter_map(|v| check_version(str_of(v, "version")).ok().map(|s| (s, v)))
                .collect();
            known.sort_by(|a, b| a.0.cmp(&b.0));
            known
                .iter()
                .rev()
                .find(|(s, _)| s.pre.is_empty())
                .or_else(|| known.last())
                .map(|(_, v)| *v)
                .ok_or_else(|| {
                    Error::Failed(format!("{publisher}/{name} has no released version yet"))
                })?
        }
    };
    let version = str_of(chosen, "version").to_owned();
    check_version(&version).map_err(|e| Error::Failed(format!("the catalogue's {e}")))?;
    let digest = Digest::parse(str_of(chosen, "digest")).map_err(|_| {
        Error::Failed(format!(
            "the catalogue names no valid digest for {publisher}/{name} {version}"
        ))
    })?;
    Ok(Resolved {
        publisher: publisher.to_owned(),
        name: name.to_owned(),
        version,
        digest,
        signer: str_of(chosen, "signer").to_owned(),
        listing_signer: str_of(ext, "signer").to_owned(),
        scopes: list_of(chosen, "scopes"),
        privileges: list_of(chosen, "privileges"),
    })
}

/// Whether a version's signer is the listing's identity, by the catalogue's own rule:
/// equal, or the listing names a release workflow up to its tag (`...@refs/tags/`) and
/// the version's identity is that workflow at a tag.
fn signed_by(listing: &str, version: &str) -> bool {
    if listing.is_empty() {
        return true;
    }
    if listing.ends_with("@refs/tags/") {
        return version
            .strip_prefix(listing)
            .is_some_and(|tag| !tag.is_empty() && !tag.contains('/'));
    }
    listing == version
}

/// The artifact, verified by `fetch` against the trust root, must be the one the
/// catalogue listed: same version, signed by the signer the catalogue names, which is
/// the listing's identity, declaring the same scopes and privileges. Any difference is
/// a refusal; nothing is installed.
pub(crate) fn matches(r: &Resolved, f: &Fetched) -> Result<(), Error> {
    let what = format!("{}/{} {}", r.publisher, r.name, r.version);
    let refuse = |why: String| Err(Error::Failed(format!("{why}: {what} was not installed")));
    if f.index != r.digest {
        return refuse(format!(
            "the registry served {} where the catalogue lists {}",
            f.index, r.digest
        ));
    }
    if f.manifest.version != r.version {
        return refuse(format!(
            "the artifact at {} is version {}, the catalogue lists {}",
            r.digest,
            clean(&f.manifest.version),
            r.version
        ));
    }
    let actual = f.verified.signer.to_string();
    if r.signer.is_empty() {
        return refuse(format!(
            "the catalogue names no signer for this version, and the artifact is signed by {actual}"
        ));
    }
    if r.signer != actual {
        return refuse(format!(
            "the artifact is signed by {actual}, but the catalogue names {}",
            clean(&r.signer)
        ));
    }
    if !signed_by(&r.listing_signer, &actual) {
        return refuse(format!(
            "the artifact is signed by {actual}, which is not the listing's identity {}",
            clean(&r.listing_signer)
        ));
    }
    let mut want_scopes = r.scopes.clone();
    let mut got_scopes = f.manifest.scopes.clone();
    want_scopes.sort();
    got_scopes.sort();
    if want_scopes != got_scopes {
        return refuse(format!(
            "the artifact asks for scopes {}, the catalogue lists {}",
            got_scopes.join(", "),
            want_scopes.join(", ")
        ));
    }
    let mut want_privs = r.privileges.clone();
    let mut got_privs = f.manifest.privileges.clone();
    want_privs.sort();
    got_privs.sort();
    if want_privs != got_privs {
        return refuse(format!(
            "the artifact declares privileges {}, the catalogue lists {}",
            if got_privs.is_empty() {
                "none".into()
            } else {
                got_privs.join(", ")
            },
            if want_privs.is_empty() {
                "none".into()
            } else {
                want_privs.join(", ")
            }
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use serde_json::json;

    use super::{At, clean, parse_at, parse_listing, pick, signed_by};
    use crate::error::Error;

    const D1: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    const D2: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";
    const D3: &str = "sha256:3333333333333333333333333333333333333333333333333333333333333333";
    const WF: &str =
        "https://github.com/inorbithr/dataplane/.github/workflows/release.yml@refs/tags/";

    fn agent() -> serde_json::Value {
        json!({"publisher": "inorbit", "name": "agent", "kind": "program", "coming": false,
               "signer": WF})
    }

    fn versions() -> Vec<serde_json::Value> {
        // Newest first, as the catalogue answers.
        vec![
            json!({"version": "0.3.0-rc.1", "digest": D3, "signer": format!("{WF}v0.3.0-rc.1"),
                   "scopes": ["agents:write"], "privileges": []}),
            json!({"version": "0.2.0", "digest": D2, "signer": format!("{WF}v0.2.0"),
                   "scopes": ["agents:write"], "privileges": []}),
            json!({"version": "0.1.0", "digest": D1, "signer": format!("{WF}v0.1.0"),
                   "scopes": [], "privileges": []}),
        ]
    }

    #[test]
    fn listings_are_publisher_slash_name() {
        assert_eq!(
            parse_listing("inorbit/agent").unwrap(),
            ("inorbit".into(), "agent".into())
        );
        for bad in [
            "agent",
            "/agent",
            "inorbit/",
            "a/agent",
            "Inorbit/agent",
            "inorbit/Agent",
            "-x/agent",
            "inorbit/agent/x",
        ] {
            assert!(matches!(parse_listing(bad), Err(Error::Usage(_))), "{bad}");
        }
    }

    #[test]
    fn newest_is_the_highest_release_not_a_release_candidate() {
        let r = pick(&agent(), &versions(), "inorbit", "agent", &At::Newest).unwrap();
        assert_eq!(r.version, "0.2.0");
        assert_eq!(r.digest.as_str(), D2);
        assert_eq!(r.signer, format!("{WF}v0.2.0"));
        assert_eq!(r.listing_signer, WF);
        // Only pre-releases: the highest of them.
        let only_pre = vec![versions()[0].clone()];
        let r = pick(&agent(), &only_pre, "inorbit", "agent", &At::Newest).unwrap();
        assert_eq!(r.version, "0.3.0-rc.1");
    }

    #[test]
    fn a_version_or_digest_is_taken_as_asked() {
        let r = pick(
            &agent(),
            &versions(),
            "inorbit",
            "agent",
            &parse_at(Some("0.1.0")).unwrap(),
        )
        .unwrap();
        assert_eq!(r.digest.as_str(), D1);
        let r = pick(
            &agent(),
            &versions(),
            "inorbit",
            "agent",
            &parse_at(Some(D3)).unwrap(),
        )
        .unwrap();
        assert_eq!(r.version, "0.3.0-rc.1");
        let e = pick(
            &agent(),
            &versions(),
            "inorbit",
            "agent",
            &parse_at(Some("9.9.9")).unwrap(),
        )
        .unwrap_err();
        assert!(e.to_string().contains("no version 9.9.9"), "{e}");
        assert!(parse_at(Some("1")).is_err());
        assert!(parse_at(Some("sha256:zz")).is_err());
    }

    #[test]
    fn coming_and_other_kinds_are_not_installed() {
        let mut coming = agent();
        coming["coming"] = json!(true);
        let e = pick(&coming, &[], "inorbit", "capture", &At::Newest).unwrap_err();
        assert!(e.to_string().contains("no released version"), "{e}");
        let mut web = agent();
        web["kind"] = json!("web");
        let e = pick(&web, &versions(), "inorbit", "agent", &At::Newest).unwrap_err();
        assert!(e.to_string().contains("programs only"), "{e}");
    }

    #[test]
    fn signer_rule_is_the_catalogues() {
        assert!(signed_by(WF, &format!("{WF}v1.0.0")));
        assert!(!signed_by(WF, WF));
        assert!(!signed_by(WF, &format!("{WF}v1/evil")));
        assert!(!signed_by(
            WF,
            "https://github.com/inorbithr/other/.github/workflows/release.yml@refs/tags/v1"
        ));
        assert!(signed_by("key:sha256:ab", "key:sha256:ab"));
        assert!(!signed_by("key:sha256:ab", "key:sha256:cd"));
        assert!(signed_by("", "anything"));
    }

    #[test]
    fn catalogue_text_cannot_reorder_or_hide_a_line() {
        assert_eq!(
            clean("ok\u{1b}[31m\u{202E}gpj.exe\u{2066}\u{200B}\u{FEFF}!"),
            "ok[31mgpj.exe!"
        );
    }
}
