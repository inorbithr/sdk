//! A narrow client for the OCI distribution API: manifests, blobs, referrers and tags,
//! read-only, from one registry (ADR 0012).
//!
//! Written here rather than taken from a crate so that what `iohr` sends, to which
//! host, stays small and visible: `GET` only, anonymous or with the one credential the
//! person gave in `IOHR_EXT_REGISTRY_AUTH`, every answer bounded in size, every
//! manifest and blob checked against its digest before it is used.

use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

use iohr_auth::Redacted;
use reqwest::StatusCode;
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use tokio::sync::Mutex;
use url::Url;

/// The registry and path prefix extensions come from unless `ext.registry` says
/// otherwise; an extension `agent` is `ghcr.io/inorbithr/iohr-ext/agent:<version>`.
pub const DEFAULT_REGISTRY: &str = "ghcr.io/inorbithr/iohr-ext";

pub(crate) const INDEX: &str = "application/vnd.oci.image.index.v1+json";
pub(crate) const MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
/// The artifact type of a Sigstore bundle attached as a referrer.
pub(crate) const BUNDLE: &str = "application/vnd.dev.sigstore.bundle.v0.3+json";

const MAX_MANIFEST: usize = 4 * 1024 * 1024;
const MAX_BUNDLE: usize = 1024 * 1024;
const MAX_REFERRERS: usize = 32;
const MAX_TAG_PAGES: usize = 10;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

/// Why the registry could not give what was asked.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OciError {
    /// The registry setting cannot be used.
    #[error("{0}")]
    Config(String),
    /// The registry could not be reached or answered with an error.
    #[error("the registry {host}: {message}")]
    Registry {
        /// The registry host.
        host: String,
        /// What happened.
        message: String,
    },
    /// The registry has no such extension or version.
    #[error("{0} is not in the registry")]
    NotFound(String),
    /// What the registry sent does not match its digest or its declared size.
    #[error("{0}")]
    Integrity(String),
}

/// A content digest: `sha256:` and 64 lower-case hex digits. No other algorithm is
/// accepted.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest(String);

impl Digest {
    /// Reads a digest.
    ///
    /// # Errors
    ///
    /// [`OciError::Integrity`] when it is not a SHA-256 digest.
    pub fn parse(s: &str) -> Result<Self, OciError> {
        let ok = s.strip_prefix("sha256:").is_some_and(|h| {
            h.len() == 64
                && h.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        });
        if ok {
            Ok(Self(s.to_owned()))
        } else {
            Err(OciError::Integrity(
                "a digest is sha256: and 64 lower-case hex digits".into(),
            ))
        }
    }

    /// The digest of `bytes`.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(format!("sha256:{}", hex(&Sha256::digest(bytes))))
    }

    /// The whole digest, `sha256:...`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The hex part.
    #[must_use]
    pub fn hex(&self) -> &str {
        &self.0[7..]
    }

    /// The first 12 hex digits, for tables and directory names.
    #[must_use]
    pub fn short(&self) -> &str {
        &self.0[7..19]
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Where extensions come from: a registry host and a path prefix, such as
/// `ghcr.io/inorbithr/iohr-ext`. `http://` is accepted for this machine only, as for
/// the API (SR-07).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    base: Url,
    prefix: String,
}

impl Source {
    /// Reads a registry setting.
    ///
    /// # Errors
    ///
    /// [`OciError::Config`] when it is not `host[:port]/path` (optionally with
    /// `https://`, or `http://` to this machine).
    pub fn parse(raw: &str) -> Result<Self, OciError> {
        let raw = raw.trim().trim_end_matches('/');
        let with_scheme = if raw.contains("://") {
            raw.to_owned()
        } else {
            format!("https://{raw}")
        };
        let bad = || {
            OciError::Config(format!(
                "`{}` is not a registry and path such as ghcr.io/inorbithr/iohr-ext",
                super::manifest::printable(raw)
            ))
        };
        let url = Url::parse(&with_scheme).map_err(|_| bad())?;
        let loopback = is_loopback(&url);
        match url.scheme() {
            "https" => {}
            "http" if loopback => {}
            "http" => {
                return Err(OciError::Config(
                    "the registry must use https (plain http only to this machine)".into(),
                ));
            }
            _ => return Err(bad()),
        }
        if url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(bad());
        }
        let prefix = url.path().trim_matches('/').to_owned();
        let path_ok = !prefix.is_empty()
            && prefix.split('/').all(|c| {
                !c.is_empty()
                    && c.bytes().all(|b| {
                        b.is_ascii_lowercase()
                            || b.is_ascii_digit()
                            || matches!(b, b'.' | b'_' | b'-')
                    })
            });
        if !path_ok {
            return Err(bad());
        }
        let mut base = url;
        base.set_path("/");
        Ok(Self { base, prefix })
    }

    /// The repository of extension `name`.
    #[must_use]
    pub fn repository(&self, name: &str) -> String {
        format!("{}/{name}", self.prefix)
    }

    /// `host[:port]/prefix`, as people write it.
    #[must_use]
    pub fn display(&self) -> String {
        let host = self.base.host_str().unwrap_or_default();
        match self.base.port() {
            Some(p) => format!("{host}:{p}/{}", self.prefix),
            None => format!("{host}/{}", self.prefix),
        }
    }

    /// As [`display`](Self::display), with `http://` kept for a registry on this
    /// machine: the form `ext.registry` stores.
    #[must_use]
    pub fn display_with_scheme(&self) -> String {
        if self.base.scheme() == "http" {
            format!("http://{}", self.display())
        } else {
            self.display()
        }
    }

    fn host(&self) -> String {
        self.base.host_str().unwrap_or_default().to_owned()
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(d)) => d == "localhost",
        None => false,
    }
}

/// An OCI descriptor: what a manifest or an index points at.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Descriptor {
    /// The media type of what it points at.
    #[serde(default)]
    pub media_type: String,
    /// The digest of what it points at.
    pub digest: String,
    /// Its size in bytes.
    pub size: u64,
    /// The platform, in an image index.
    #[serde(default)]
    pub platform: Option<Platform>,
    /// The artifact type, in a referrers index.
    #[serde(default)]
    pub artifact_type: Option<String>,
}

/// An image index entry's platform.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct Platform {
    /// `linux`, `darwin` or `windows`.
    pub os: String,
    /// `amd64` or `arm64`.
    pub architecture: String,
}

impl Platform {
    /// This machine, in OCI's words.
    #[must_use]
    pub fn current() -> Self {
        let os = match std::env::consts::OS {
            "macos" => "darwin",
            other => other,
        };
        let architecture = match std::env::consts::ARCH {
            "x86_64" => "amd64",
            "aarch64" => "arm64",
            other => other,
        };
        Self {
            os: os.into(),
            architecture: architecture.into(),
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.os, self.architecture)
    }
}

/// An image index: one manifest per platform.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Index {
    /// Its media type, when it says.
    #[serde(default)]
    pub media_type: Option<String>,
    /// The manifests it lists.
    #[serde(default)]
    pub manifests: Vec<Descriptor>,
}

/// An image manifest: a config and layers.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ImageManifest {
    /// Its media type, when it says.
    #[serde(default)]
    pub media_type: Option<String>,
    /// The artifact type, for an artifact such as a signature bundle.
    #[serde(default)]
    pub artifact_type: Option<String>,
    /// The config blob.
    pub config: Descriptor,
    /// The layers.
    #[serde(default)]
    pub layers: Vec<Descriptor>,
    /// The manifest this one refers to, for a referrer.
    #[serde(default)]
    pub subject: Option<Descriptor>,
}

/// A manifest as fetched: its bytes, checked against `digest`.
#[derive(Debug)]
pub(crate) struct Fetched {
    pub(crate) media_type: String,
    pub(crate) digest: Digest,
    pub(crate) bytes: Vec<u8>,
}

/// A read-only client for one registry.
pub struct Registry {
    http: reqwest::Client,
    source: Source,
    auth: Option<Redacted<String>>,
    tokens: Mutex<HashMap<String, Redacted<String>>>,
    verbose: bool,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry")
            .field("source", &self.source.display())
            .finish_non_exhaustive()
    }
}

impl Registry {
    /// A client for `source`. `auth` is `user:password` for a registry that needs one
    /// (`IOHR_EXT_REGISTRY_AUTH`), sent only to that registry and its token service.
    ///
    /// # Errors
    ///
    /// [`OciError::Config`] when the HTTP stack cannot start.
    pub fn new(
        source: Source,
        auth: Option<Redacted<String>>,
        verbose: bool,
    ) -> Result<Self, OciError> {
        let origin = source.base.origin();
        let redirects = reqwest::redirect::Policy::custom(move |attempt| {
            // Blob downloads may be redirected to the registry's storage: follow at
            // most 5, and only to https (or back to this machine's own registry).
            if attempt.previous().len() >= 5 {
                attempt.error("too many redirects")
            } else if attempt.url().scheme() == "https" || attempt.url().origin() == origin {
                attempt.follow()
            } else {
                attempt.stop()
            }
        });
        let http = reqwest::Client::builder()
            .user_agent(format!("iohr/{}", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(redirects)
            .https_only(source.base.scheme() == "https")
            .build()
            .map_err(|e| OciError::Config(format!("cannot start the HTTP client: {e}")))?;
        Ok(Self {
            http,
            source,
            auth,
            tokens: Mutex::new(HashMap::new()),
            verbose,
        })
    }

    /// Where this client reads from.
    #[must_use]
    pub fn source(&self) -> &Source {
        &self.source
    }

    /// A manifest or index by tag or digest, checked against the digest when one was
    /// asked for.
    pub(crate) async fn manifest(&self, repo: &str, reference: &str) -> Result<Fetched, OciError> {
        let accept = format!("{INDEX}, {MANIFEST}");
        let (status, headers, bytes) = self
            .get(
                repo,
                &format!("manifests/{reference}"),
                Some(&accept),
                MAX_MANIFEST,
            )
            .await?;
        if status == StatusCode::NOT_FOUND {
            return Err(OciError::NotFound(format!("{repo}:{reference}")));
        }
        self.ok(status)?;
        let digest = Digest::of(&bytes);
        if reference.starts_with("sha256:") && digest.as_str() != reference {
            return Err(OciError::Integrity(format!(
                "the registry sent a manifest for {reference} whose digest is {digest}"
            )));
        }
        let media_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap_or_default().trim().to_owned())
            .unwrap_or_default();
        Ok(Fetched {
            media_type,
            digest,
            bytes,
        })
    }

    /// A blob, at most `max` bytes, refused unless its size and digest are the ones
    /// the descriptor names.
    pub(crate) async fn blob(
        &self,
        repo: &str,
        desc: &Descriptor,
        max: usize,
    ) -> Result<Vec<u8>, OciError> {
        let digest = Digest::parse(&desc.digest)?;
        let size = usize::try_from(desc.size).unwrap_or(usize::MAX);
        if size > max {
            return Err(OciError::Integrity(format!(
                "{} is {} bytes, more than the {} allowed",
                digest, desc.size, max
            )));
        }
        let (status, _, bytes) = self
            .get(repo, &format!("blobs/{digest}"), None, size)
            .await?;
        if status == StatusCode::NOT_FOUND {
            return Err(OciError::NotFound(format!("{repo}@{digest}")));
        }
        self.ok(status)?;
        if bytes.len() != size || Digest::of(&bytes) != digest {
            return Err(OciError::Integrity(format!(
                "the registry sent a blob that does not match {digest}: refusing it"
            )));
        }
        Ok(bytes)
    }

    /// The Sigstore bundles attached to `digest` as referrers (OCI 1.1), through the
    /// referrers API or, where a registry has none, the referrers tag schema. Each comes
    /// with the digest it says it refers to.
    pub(crate) async fn bundles(
        &self,
        repo: &str,
        digest: &Digest,
    ) -> Result<Vec<Vec<u8>>, OciError> {
        let accept = INDEX.to_owned();
        let (status, _, bytes) = self
            .get(
                repo,
                &format!("referrers/{digest}?artifactType={BUNDLE}"),
                Some(&accept),
                MAX_MANIFEST,
            )
            .await?;
        let bytes = if status == StatusCode::NOT_FOUND {
            match self
                .manifest(repo, &format!("sha256-{}", digest.hex()))
                .await
            {
                Ok(f) => f.bytes,
                Err(OciError::NotFound(_)) => return Ok(Vec::new()),
                Err(e) => return Err(e),
            }
        } else {
            self.ok(status)?;
            bytes
        };
        let index: Index = serde_json::from_slice(&bytes).map_err(|e| self.bad(&e))?;
        let mut out = Vec::new();
        for d in index
            .manifests
            .iter()
            .filter(|d| d.artifact_type.as_deref() == Some(BUNDLE))
            .take(MAX_REFERRERS)
        {
            let m = self.manifest(repo, &Digest::parse(&d.digest)?.0).await?;
            let manifest: ImageManifest =
                serde_json::from_slice(&m.bytes).map_err(|e| self.bad(&e))?;
            // A referrer that names another subject is not about this artifact.
            if manifest
                .subject
                .as_ref()
                .is_some_and(|s| s.digest != digest.as_str())
            {
                continue;
            }
            if let Some(layer) = manifest.layers.iter().find(|l| l.media_type == BUNDLE) {
                out.push(self.blob(repo, layer, MAX_BUNDLE).await?);
            }
        }
        Ok(out)
    }

    /// Every tag of `repo`, following at most 10 pages.
    pub(crate) async fn tags(&self, repo: &str) -> Result<Vec<String>, OciError> {
        #[derive(Deserialize)]
        struct Tags {
            #[serde(default)]
            tags: Option<Vec<String>>,
        }
        let mut out = Vec::new();
        let mut last: Option<String> = None;
        for _ in 0..MAX_TAG_PAGES {
            let path = match &last {
                Some(l) => format!("tags/list?n=1000&last={l}"),
                None => "tags/list?n=1000".to_owned(),
            };
            let (status, headers, bytes) = self.get(repo, &path, None, MAX_MANIFEST).await?;
            if status == StatusCode::NOT_FOUND {
                return Err(OciError::NotFound(repo.to_owned()));
            }
            self.ok(status)?;
            let page: Tags = serde_json::from_slice(&bytes).map_err(|e| self.bad(&e))?;
            let page = page.tags.unwrap_or_default();
            let more = headers.contains_key(header::LINK) && !page.is_empty();
            last = page.last().cloned();
            out.extend(page);
            if !more {
                break;
            }
        }
        Ok(out)
    }

    fn ok(&self, status: StatusCode) -> Result<(), OciError> {
        if status.is_success() {
            return Ok(());
        }
        let message = match status.as_u16() {
            401 | 403 => "refused access; a private mirror needs IOHR_EXT_REGISTRY_AUTH".to_owned(),
            429 => "asked to slow down (HTTP 429); try again shortly".to_owned(),
            s => format!("answered HTTP {s}"),
        };
        Err(OciError::Registry {
            host: self.source.host(),
            message,
        })
    }

    fn bad(&self, e: &serde_json::Error) -> OciError {
        OciError::Registry {
            host: self.source.host(),
            message: format!("sent something that is not an OCI document: {e}"),
        }
    }

    /// One `GET` on `/v2/<repo>/<rest>`, authenticating once when challenged.
    async fn get(
        &self,
        repo: &str,
        rest: &str,
        accept: Option<&str>,
        max: usize,
    ) -> Result<(StatusCode, HeaderMap, Vec<u8>), OciError> {
        let url = self
            .source
            .base
            .join(&format!("v2/{repo}/{rest}"))
            .map_err(|e| OciError::Config(e.to_string()))?;
        let mut challenged = false;
        loop {
            let mut req = self.http.get(url.clone());
            if let Some(a) = accept {
                req = req.header(header::ACCEPT, a);
            }
            if let Some(t) = self.tokens.lock().await.get(repo) {
                let mut v = HeaderValue::from_str(&format!("Bearer {}", t.expose()))
                    .map_err(|_| self.refused("a token the registry sent is not a header value"))?;
                v.set_sensitive(true);
                req = req.header(header::AUTHORIZATION, v);
            } else if challenged && let Some(a) = &self.auth {
                req = req.header(header::AUTHORIZATION, basic(a)?);
            }
            let started = Instant::now();
            let reply = req.send().await.map_err(|e| self.unreachable(&e))?;
            let status = reply.status();
            if self.verbose {
                crate::context::note(&format!(
                    "GET {} /v2/{repo}/{} -> {} in {} ms",
                    self.source.host(),
                    rest.split('?').next().unwrap_or_default(),
                    status.as_u16(),
                    started.elapsed().as_millis()
                ));
            }
            if status == StatusCode::UNAUTHORIZED && !challenged {
                challenged = true;
                let challenge = reply
                    .headers()
                    .get(header::WWW_AUTHENTICATE)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned)
                    .unwrap_or_default();
                if let Some(params) = challenge.strip_prefix("Bearer ") {
                    let token = self.token(&parse_challenge(params), repo).await?;
                    self.tokens.lock().await.insert(repo.to_owned(), token);
                }
                continue;
            }
            let headers = reply.headers().clone();
            let body = read_capped(reply, max).await.map_err(|e| match e {
                Capped::TooLarge => OciError::Integrity(format!(
                    "the registry sent more than {max} bytes for {rest}: refusing it"
                )),
                Capped::Http(e) => self.unreachable(&e),
            })?;
            return Ok((status, headers, body));
        }
    }

    /// A pull token from the registry's token service (Docker token protocol):
    /// anonymous, or with the configured credential.
    async fn token(
        &self,
        params: &HashMap<String, String>,
        repo: &str,
    ) -> Result<Redacted<String>, OciError> {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(default)]
            token: Option<String>,
            #[serde(default)]
            access_token: Option<String>,
        }
        let realm = params
            .get("realm")
            .and_then(|r| Url::parse(r).ok())
            .filter(|r| r.scheme() == "https" || (r.scheme() == "http" && is_loopback(r)))
            .ok_or_else(|| self.refused("named a token service that is not https"))?;
        let scope = params
            .get("scope")
            .cloned()
            .unwrap_or_else(|| format!("repository:{repo}:pull"));
        let mut query: Vec<(&str, &str)> = vec![("scope", &scope)];
        if let Some(s) = params.get("service") {
            query.push(("service", s));
        }
        let mut req = self.http.get(realm).query(&query);
        if let Some(a) = &self.auth {
            req = req.header(header::AUTHORIZATION, basic(a)?);
        }
        let reply = req.send().await.map_err(|e| self.unreachable(&e))?;
        let status = reply.status();
        let body = read_capped(reply, 64 * 1024).await.map_err(|e| match e {
            Capped::TooLarge => self.refused("sent an oversized token"),
            Capped::Http(e) => self.unreachable(&e),
        })?;
        if !status.is_success() {
            return Err(OciError::Registry {
                host: self.source.host(),
                message: format!(
                    "refused a pull token (HTTP {}); a private mirror needs IOHR_EXT_REGISTRY_AUTH",
                    status.as_u16()
                ),
            });
        }
        let w: Wire = serde_json::from_slice(&body).map_err(|e| self.bad(&e))?;
        w.token
            .or(w.access_token)
            .filter(|t| !t.is_empty())
            .map(Redacted::new)
            .ok_or_else(|| self.refused("sent no pull token"))
    }

    fn refused(&self, what: &str) -> OciError {
        OciError::Registry {
            host: self.source.host(),
            message: what.to_owned(),
        }
    }

    fn unreachable(&self, e: &reqwest::Error) -> OciError {
        let mut parts = Vec::new();
        let mut source: Option<&dyn std::error::Error> = std::error::Error::source(e);
        while let Some(s) = source {
            parts.push(s.to_string());
            source = s.source();
        }
        OciError::Registry {
            host: self.source.host(),
            message: if e.is_timeout() {
                "did not answer in time".to_owned()
            } else if parts.is_empty() {
                "could not be reached".to_owned()
            } else {
                format!("could not be reached: {}", parts.join(": "))
            },
        }
    }
}

fn basic(auth: &Redacted<String>) -> Result<HeaderValue, OciError> {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(auth.expose());
    let mut v = HeaderValue::from_str(&format!("Basic {encoded}"))
        .map_err(|_| OciError::Config("IOHR_EXT_REGISTRY_AUTH is not user:password".into()))?;
    v.set_sensitive(true);
    Ok(v)
}

enum Capped {
    TooLarge,
    Http(reqwest::Error),
}

async fn read_capped(mut resp: reqwest::Response, max: usize) -> Result<Vec<u8>, Capped> {
    if resp
        .content_length()
        .is_some_and(|n| n > u64::try_from(max).unwrap_or(u64::MAX))
    {
        return Err(Capped::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(Capped::Http)? {
        if body.len() + chunk.len() > max {
            return Err(Capped::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// The `key="value"` pairs of a `WWW-Authenticate: Bearer` challenge.
pub(crate) fn parse_challenge(params: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut rest = params.trim();
    while !rest.is_empty() {
        let Some((key, after)) = rest.split_once('=') else {
            break;
        };
        let key = key
            .trim()
            .trim_start_matches(',')
            .trim()
            .to_ascii_lowercase();
        let after = after.trim_start();
        let (value, next) = if let Some(quoted) = after.strip_prefix('"') {
            match quoted.find('"') {
                Some(end) => (&quoted[..end], &quoted[end + 1..]),
                None => (quoted, ""),
            }
        } else {
            match after.find(',') {
                Some(end) => (&after[..end], &after[end..]),
                None => (after, ""),
            }
        };
        out.insert(key, value.to_owned());
        rest = next.trim_start().trim_start_matches(',').trim_start();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Digest, Source, parse_challenge};

    #[test]
    fn sources() {
        let s = Source::parse("ghcr.io/inorbithr/iohr-ext").unwrap();
        assert_eq!(s.repository("agent"), "inorbithr/iohr-ext/agent");
        assert_eq!(s.display(), "ghcr.io/inorbithr/iohr-ext");
        assert!(Source::parse("http://127.0.0.1:5000/x/").is_ok());
        assert!(Source::parse("artifactory.acme.hr:8443/docker-remote/inorbithr").is_ok());
        for bad in [
            "http://registry.acme.hr/x",
            "ghcr.io",
            "ghcr.io/",
            "ghcr.io/UPPER",
            "ftp://ghcr.io/x",
            "https://u:p@ghcr.io/x",
            "ghcr.io/x?y=1",
        ] {
            assert!(Source::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn digests() {
        let d = Digest::of(b"");
        assert_eq!(
            d.as_str(),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(d.short(), "e3b0c44298fc");
        assert!(Digest::parse(d.as_str()).is_ok());
        for bad in [
            "sha512:00",
            "sha256:E3B0",
            "e3b0c44298fc",
            "sha256:../../etc",
        ] {
            assert!(Digest::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn challenges() {
        let c = parse_challenge(
            r#"realm="https://ghcr.io/token",service="ghcr.io",scope="repository:a/b:pull,push""#,
        );
        assert_eq!(c["realm"], "https://ghcr.io/token");
        assert_eq!(c["service"], "ghcr.io");
        assert_eq!(c["scope"], "repository:a/b:pull,push");
        let c = parse_challenge("realm=https://r.example/t, service=r");
        assert_eq!(c["realm"], "https://r.example/t");
        assert_eq!(c["service"], "r");
        assert!(parse_challenge("").is_empty());
    }
}
