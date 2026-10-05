//! Which proxy a request goes through (`docs/config.md` section 6.2): the SDK's own
//! `no_proxy` matcher, the same in every runtime, pinned by
//! `conformance/vectors/no-proxy/`.

use std::net::IpAddr;

use url::Url;

use super::resolve::{is_loopback, no_proxy_entry};

/// One `no_proxy` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    /// `*`: every host.
    All,
    /// A name and every subdomain, on any port or on one.
    Domain(String, Option<u16>),
    /// One address.
    Ip(IpAddr, Option<u16>),
    /// Addresses written as IP literals in a range.
    Cidr(IpAddr, u8),
}

/// The proxy, whether it was set explicitly, and the hosts that skip it.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProxyRules {
    proxy: Option<Url>,
    /// Code, `INORBIT_PROXY` or the file: applies to loopback too.
    explicit: bool,
    entries: Vec<Entry>,
}

impl ProxyRules {
    /// The rules for `proxy` (a URL; `None` for none) and the `no_proxy` entries. The
    /// error names the setting (`proxy` or `no_proxy`) and what is wrong.
    pub(crate) fn new(
        proxy: Option<&str>,
        explicit: bool,
        no_proxy: &[String],
    ) -> Result<Self, (&'static str, String)> {
        let proxy = match proxy {
            None => None,
            Some(p) => {
                let u = Url::parse(p).map_err(|_| {
                    (
                        "proxy",
                        format!(
                            "{:?} is not an absolute URL",
                            super::resolve::redact_userinfo(p)
                        ),
                    )
                })?;
                if u.scheme() != "http" && u.scheme() != "https" {
                    return Err((
                        "proxy",
                        "must be an http:// or https:// proxy URL, or off".to_owned(),
                    ));
                }
                Some(u)
            }
        };
        let mut entries = Vec::new();
        for raw in no_proxy {
            let e = raw.trim().to_ascii_lowercase();
            if e.is_empty() {
                continue;
            }
            if !no_proxy_entry(&e) {
                return Err((
                    "no_proxy",
                    format!(
                        "{e:?} is not a no_proxy entry: a host, .domain, host:port, an IP address or a CIDR range"
                    ),
                ));
            }
            entries.push(parse(&e));
        }
        Ok(Self {
            proxy,
            explicit,
            entries,
        })
    }

    /// Whether any proxy is configured.
    pub(crate) fn is_set(&self) -> bool {
        self.proxy.is_some()
    }

    /// The proxy for a request to `url`, or `None` to connect directly.
    pub(crate) fn proxy_for(&self, url: &Url) -> Option<Url> {
        let proxy = self.proxy.as_ref()?;
        let host = match url.host()? {
            url::Host::Domain(d) => Host::Name(d.to_ascii_lowercase()),
            url::Host::Ipv4(ip) => Host::Ip(IpAddr::V4(ip)),
            url::Host::Ipv6(ip) => Host::Ip(IpAddr::V6(ip)),
        };
        let port = url.port_or_known_default();
        if !self.explicit && url.host_str().is_some_and(is_loopback) {
            return None;
        }
        let skip = self.entries.iter().any(|e| matches(e, &host, port));
        (!skip).then(|| proxy.clone())
    }
}

enum Host {
    Name(String),
    Ip(IpAddr),
}

fn matches(entry: &Entry, host: &Host, port: Option<u16>) -> bool {
    let port_ok = |p: &Option<u16>| p.is_none() || *p == port;
    match (entry, host) {
        (Entry::All, _) => true,
        (Entry::Domain(d, p), Host::Name(h)) => {
            port_ok(p) && (h == d || h.strip_suffix(d.as_str()).is_some_and(|r| r.ends_with('.')))
        }
        (Entry::Ip(a, p), Host::Ip(h)) => port_ok(p) && a == h,
        (Entry::Cidr(net, bits), Host::Ip(h)) => in_range(*net, *bits, *h),
        _ => false,
    }
}

fn in_range(net: IpAddr, bits: u8, ip: IpAddr) -> bool {
    match (net, ip) {
        (IpAddr::V4(n), IpAddr::V4(i)) => {
            let mask = if bits == 0 {
                0
            } else {
                u32::MAX << (32 - u32::from(bits))
            };
            (u32::from(n) & mask) == (u32::from(i) & mask)
        }
        (IpAddr::V6(n), IpAddr::V6(i)) => {
            let mask = if bits == 0 {
                0
            } else {
                u128::MAX << (128 - u32::from(bits))
            };
            (u128::from(n) & mask) == (u128::from(i) & mask)
        }
        _ => false,
    }
}

/// Parses an entry `no_proxy_entry` accepted.
fn parse(e: &str) -> Entry {
    if e == "*" {
        return Entry::All;
    }
    if let Some((ip, bits)) = e.split_once('/')
        && let (Ok(ip), Ok(bits)) = (ip.parse::<IpAddr>(), bits.parse::<u8>())
    {
        return Entry::Cidr(ip, bits);
    }
    let bare = e.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return Entry::Ip(ip, None);
    }
    let (host, port) = match e.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') || h.ends_with(']') => (h, p.parse::<u16>().ok()),
        _ => (e, None),
    };
    let host = host.trim_start_matches('.');
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    match bare.parse::<IpAddr>() {
        Ok(ip) => Entry::Ip(ip, port),
        Err(_) => Entry::Domain(host.to_owned(), port),
    }
}

#[cfg(test)]
mod tests {
    use super::ProxyRules;

    fn via(no_proxy: &[&str], url: &str) -> bool {
        let list: Vec<String> = no_proxy.iter().map(|s| (*s).to_owned()).collect();
        ProxyRules::new(Some("http://proxy.example:3128"), false, &list)
            .unwrap()
            .proxy_for(&url::Url::parse(url).unwrap())
            .is_some()
    }

    #[test]
    fn names_ports_and_ranges() {
        assert!(!via(&["example.com"], "https://a.example.com/"));
        assert!(via(&["example.com"], "https://notexample.com/"));
        assert!(!via(&["10.0.0.0/8"], "https://10.9.9.9/"));
        assert!(via(&["example.com:8443"], "https://example.com/"));
        assert!(!via(&[], "https://localhost:1/"));
    }
}
