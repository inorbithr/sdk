//! `iohr domains`: prove an account controls a domain with one DNS TXT record
//! (platform RFC 0030).

use std::time::{Duration, Instant};

use inorbithr::Method;
use serde_json::Value;

use crate::Env;
use crate::cli::{DomainsCommand, Global};
use crate::context::{Session, account, session};
use crate::error::Error;
use crate::output::Out;

const POLL: Duration = Duration::from_secs(10);

pub(crate) async fn run(g: &Global, env: &Env, cmd: DomainsCommand, out: Out) -> Result<(), Error> {
    let s = session(g, env).await?;
    match cmd {
        DomainsCommand::Add {
            domain,
            subdomain,
            account: acc,
        } => {
            add(
                &s,
                &account(&s, acc)?,
                &domain_name(&domain)?,
                subdomain,
                out,
            )
            .await
        }
        DomainsCommand::Verify {
            domain,
            wait,
            timeout,
            account: acc,
        } => {
            let deadline = wait.then(|| Duration::from_secs(timeout));
            verify(
                &s,
                &account(&s, acc)?,
                &domain_name(&domain)?,
                deadline,
                out,
            )
            .await
        }
        DomainsCommand::Confirm {
            domain,
            account: acc,
        } => confirm(&s, &account(&s, acc)?, &domain_name(&domain)?, out).await,
        DomainsCommand::List { account: acc } => list(&s, &account(&s, acc)?, out).await,
        DomainsCommand::Rm {
            domain,
            account: acc,
        } => rm(&s, &account(&s, acc)?, &domain_name(&domain)?, out).await,
    }
}

/// A domain name as the API takes it: lower case, ASCII (IDNs in their `xn--` form),
/// at least two labels of 1 to 63 letters, digits and `-`, at most 253 characters.
pub(crate) fn domain_name(raw: &str) -> Result<String, Error> {
    let d = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = d.split('.').collect();
    let ok = (1..=253).contains(&d.len())
        && labels.len() >= 2
        && labels.iter().all(|l| {
            (1..=63).contains(&l.len())
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        && labels
            .last()
            .is_some_and(|tld| !tld.bytes().all(|b| b.is_ascii_digit()));
    if ok {
        Ok(d)
    } else {
        Err(Error::Usage(
            "give a domain name such as acme.hr (an internationalised name in its xn-- form)"
                .into(),
        ))
    }
}

fn path(acc: &str, rest: &str) -> String {
    format!("/v1/accounts/orgs/{acc}/domains{rest}")
}

fn scoped(e: inorbithr::Error) -> Error {
    if e.status() == Some(403) {
        Error::with_hint(
            e,
            "Domains need the domains:read scope to list and domains:write to change, and an \
             owner or admin of the account.",
        )
    } else {
        e.into()
    }
}

fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or_default()
}

async fn add(s: &Session, acc: &str, domain: &str, subdomain: bool, out: Out) -> Result<(), Error> {
    let body = serde_json::json!({
        "domain": domain,
        "scope": if subdomain { "subdomain" } else { "domain" },
    });
    let made: Value = s
        .api
        .send(Method::Post, &path(acc, ""), &[], Some(&body))
        .await
        .and_then(|r| r.json())
        .map_err(scoped)?;
    let d = made.get("domain").cloned().unwrap_or(Value::Null);
    if out.json {
        Out::print_json(&d);
        return Ok(());
    }
    Out::note(&format!(
        "Add this TXT record at your DNS provider, then run `iohr domains verify {domain} --wait`:"
    ));
    Out::pairs(&[
        ("name", str_of(&d, "record_name").to_owned()),
        ("type", "TXT".to_owned()),
        ("value", str_of(&d, "record_value").to_owned()),
    ]);
    let expires = str_of(&d, "expires_at");
    if !expires.is_empty() {
        Out::note(&format!(
            "The token expires {} unless the domain is verified by then.",
            super::day(expires)
        ));
    }
    Ok(())
}

async fn check(s: &Session, acc: &str, domain: &str) -> Result<Value, Error> {
    s.api
        .send(
            Method::Post,
            &path(acc, &format!("/{domain}/check")),
            &[],
            None,
        )
        .await
        .and_then(|r| r.json())
        .map_err(scoped)
}

async fn verify(
    s: &Session,
    acc: &str,
    domain: &str,
    wait: Option<Duration>,
    out: Out,
) -> Result<(), Error> {
    let started = Instant::now();
    let mut result = check(s, acc, domain).await?;
    if let Some(limit) = wait {
        while str_of(&result, "status") != "seen" {
            if started.elapsed() + POLL > limit {
                print_check(domain, &result, out);
                return Err(Error::Failed(format!(
                    "the record for {domain} was not seen within {} s; DNS changes can take a while, run this again later",
                    limit.as_secs()
                )));
            }
            if !out.json {
                Out::note(&format!(
                    "{domain}: {}; checking again in {} s",
                    describe(str_of(&result, "status")),
                    POLL.as_secs()
                ));
            }
            tokio::time::sleep(POLL).await;
            result = check(s, acc, domain).await?;
        }
    }
    print_check(domain, &result, out);
    if str_of(&result, "status") == "seen" && !out.json {
        Out::note(&format!(
            "Run `iohr domains confirm {domain}` to mark it verified."
        ));
    }
    Ok(())
}

fn describe(status: &str) -> &str {
    match status {
        "pending" => "no resolver sees the record yet",
        "wrong_value" => "a record is there with another value",
        "partial" => "some resolvers see it, not enough yet",
        "seen" => "seen by enough resolvers",
        other => other,
    }
}

fn print_check(domain: &str, result: &Value, out: Out) {
    if out.json {
        Out::print_json(result);
        return;
    }
    Out::note(&format!("{domain}: {}", describe(str_of(result, "status"))));
    let rows: Vec<Vec<String>> = result
        .get("resolvers")
        .and_then(Value::as_array)
        .map(|rs| {
            rs.iter()
                .map(|r| {
                    vec![
                        str_of(r, "name").to_owned(),
                        if r.get("seen").and_then(Value::as_bool).unwrap_or(false) {
                            "yes".into()
                        } else {
                            "no".into()
                        },
                        str_of(r, "value").to_owned(),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    if !rows.is_empty() {
        Out::table(&["RESOLVER", "SEEN", "VALUE"], &rows);
    }
}

async fn confirm(s: &Session, acc: &str, domain: &str, out: Out) -> Result<(), Error> {
    let v: Value = s
        .api
        .send(
            Method::Post,
            &path(acc, &format!("/{domain}/confirm")),
            &[],
            None,
        )
        .await
        .and_then(|r| r.json())
        .map_err(scoped)?;
    let d = v.get("domain").cloned().unwrap_or(Value::Null);
    if out.json {
        Out::print_json(&d);
    } else {
        Out::note(&format!(
            "{domain} is {} for this account. Keep the record: it is checked again every day.",
            str_of(&d, "status")
        ));
    }
    Ok(())
}

async fn list(s: &Session, acc: &str, out: Out) -> Result<(), Error> {
    let v: Value = s.api.get(&path(acc, ""), &[]).await.map_err(scoped)?;
    let domains = v
        .get("domains")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if out.json {
        Out::print_json(&domains);
        return Ok(());
    }
    let rows: Vec<Vec<String>> = domains
        .iter()
        .map(|d| {
            vec![
                str_of(d, "domain").to_owned(),
                str_of(d, "scope").to_owned(),
                str_of(d, "status").to_owned(),
                super::day(str_of(d, "verified_at")),
                super::day(str_of(d, "last_checked_at")),
            ]
        })
        .collect();
    Out::table(
        &["DOMAIN", "SCOPE", "STATUS", "VERIFIED", "LAST CHECKED"],
        &rows,
    );
    Ok(())
}

async fn rm(s: &Session, acc: &str, domain: &str, out: Out) -> Result<(), Error> {
    s.api
        .send(Method::Delete, &path(acc, &format!("/{domain}")), &[], None)
        .await
        .map_err(scoped)?;
    if out.json {
        Out::print_json(&serde_json::json!({ "domain": domain, "removed": true }));
    } else {
        Out::note(&format!(
            "Removed {domain}. You may now delete its _inorbit-verify TXT record."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::domain_name;

    #[test]
    fn domain_names() {
        assert_eq!(domain_name("Acme.HR.").ok().as_deref(), Some("acme.hr"));
        assert!(domain_name("staging.acme.hr").is_ok());
        assert!(domain_name("xn--mnchen-3ya.de").is_ok());
        for bad in [
            "acme", "-a.hr", "a-.hr", "a..hr", "a/b.hr", "1.2.3.4", "ä.hr", "a b.hr", "",
        ] {
            assert!(domain_name(bad).is_err(), "{bad}");
        }
    }
}
