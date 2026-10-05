//! The transport under the pipeline (`docs/config.md` section 6): one HTTP client per
//! client, with the connect timeout, the proxy and `no_proxy` rules, trust (the system
//! store plus `ca_bundle`, or the bundle alone), a client certificate and pinned keys;
//! and the `/v1/ws` upgrade over the same proxy and TLS settings.

#[cfg(feature = "rustls")]
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use url::Url;

use crate::auth::transport_reason;
use crate::config::{ProxyRules, Settings};
use crate::error::{ConfigError, Error, Headers, MAX_BODY};
use crate::middleware::{Body, Request, Response};
use crate::socket::Io;

/// Sends requests: the innermost step of every pipeline.
pub(crate) struct Transport {
    pub(crate) http: reqwest::Client,
    proxy: ProxyRules,
    connect_timeout: Duration,
    /// The caller supplied `http`: its redirects are its own, so a `3xx` it followed
    /// to another origin is refused here.
    caller: bool,
    #[cfg(feature = "rustls")]
    tls: Arc<rustls::ClientConfig>,
}

impl Transport {
    /// The transport for `settings`, or around the caller's own client.
    pub(crate) fn new(
        settings: &Settings,
        caller: Option<reqwest::Client>,
        https_only: bool,
    ) -> Result<Self, ConfigError> {
        #[cfg(feature = "rustls")]
        let tls = Arc::new(if caller.is_some() {
            tls::config(&Settings::plain_tls())?
        } else {
            tls::config(settings)?
        });
        let (http, is_caller) = if let Some(c) = caller {
            (c, true)
        } else {
            {
                let mut b = reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .connect_timeout(settings.connect_timeout)
                    .no_proxy();
                if settings.proxy.is_set() {
                    let rules = settings.proxy.clone();
                    b = b.proxy(reqwest::Proxy::custom(move |u| rules.proxy_for(u)));
                }
                #[cfg(feature = "rustls")]
                {
                    b = b.tls_backend_preconfigured((*tls).clone());
                }
                if https_only {
                    b = b.https_only(true);
                }
                (
                    b.build().map_err(|e| ConfigError::Http(e.to_string()))?,
                    false,
                )
            }
        };
        Ok(Self {
            http,
            proxy: if is_caller {
                ProxyRules::default()
            } else {
                settings.proxy.clone()
            },
            connect_timeout: settings.connect_timeout,
            caller: is_caller,
            #[cfg(feature = "rustls")]
            tls,
        })
    }

    /// Sends one request; reads the whole body unless the answer is a stream that
    /// opened.
    pub(crate) async fn send(&self, req: Request) -> Result<Response, Error> {
        if req.info.upgrade {
            return self.upgrade(req).await;
        }
        let host = req.url.host_str().unwrap_or_default().to_owned();
        let mut headers = reqwest::header::HeaderMap::new();
        for (k, v) in req.headers.iter() {
            let name = reqwest::header::HeaderName::from_bytes(k.as_bytes());
            let value = reqwest::header::HeaderValue::from_str(v);
            match (name, value) {
                (Ok(n), Ok(v)) => {
                    headers.append(n, v);
                }
                _ => {
                    return Err(ConfigError::Http(format!("{k} is not a usable header")).into());
                }
            }
        }
        let mut rb = self
            .http
            .request(req.method.as_reqwest(), req.url.clone())
            .headers(headers);
        if let Some(b) = req.body {
            rb = rb.body(b);
        }
        let mut resp = rb.send().await.map_err(|e| failed(&host, &e))?;
        if self.caller && resp.url().origin() != req.url.origin() {
            return Err(Error::Connection {
                host,
                reason: "the HTTP client followed a redirect to another origin; refusing it".into(),
            });
        }
        let status = resp.status().as_u16();
        let headers = Headers::new(resp.headers().iter().map(|(k, v)| {
            (
                k.as_str().to_owned(),
                v.to_str().unwrap_or_default().to_owned(),
            )
        }));
        if req.info.stream && (200..300).contains(&status) {
            return Ok(Response {
                status,
                headers,
                body: Body::Stream(resp),
                rate_limit: None,
            });
        }
        if resp.content_length().is_some_and(|n| n > MAX_BODY as u64) {
            return Err(Error::TooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| Error::Connection {
            host: host.clone(),
            reason: transport_reason(&e),
        })? {
            if body.len() + chunk.len() > MAX_BODY {
                return Err(Error::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Response {
            status,
            headers,
            body: Body::Bytes(body),
            rate_limit: None,
        })
    }

    /// The `/v1/ws` upgrade: TCP (through the proxy when one applies), TLS, then the
    /// WebSocket handshake with the request's headers. A refusal is an answer like any
    /// other; the socket comes back as the body of a `101`.
    async fn upgrade(&self, req: Request) -> Result<Response, Error> {
        use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest as _};
        let url = req.url.clone();
        let secure = url.scheme() == "https";
        let host = url.host_str().unwrap_or_default().to_owned();
        let bare = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned();
        let port = url
            .port_or_known_default()
            .unwrap_or(if secure { 443 } else { 80 });
        let failed = |reason: String| Error::Connection {
            host: host.clone(),
            reason,
        };
        let mut ws_url = url.clone();
        let _ = ws_url.set_scheme(if secure { "wss" } else { "ws" });
        let mut request =
            ws_url
                .as_str()
                .into_client_request()
                .map_err(|e| ConfigError::InvalidUrl {
                    what: "base_url",
                    reason: e.to_string(),
                })?;
        for (k, v) in req.headers.iter() {
            let name = tungstenite::http::HeaderName::from_bytes(k.as_bytes());
            let value = tungstenite::http::HeaderValue::from_str(v);
            if let (Ok(n), Ok(v)) = (name, value) {
                request.headers_mut().insert(n, v);
            }
        }
        let connect = async {
            let io: Box<dyn Io> = if let Some(proxy) = self.proxy.proxy_for(&url) {
                self.tunnel(&proxy, &host, port).await?
            } else {
                {
                    let tcp = tokio::net::TcpStream::connect((bare.as_str(), port))
                        .await
                        .map_err(|e| failed(e.to_string()))?;
                    let _ = tcp.set_nodelay(true);
                    Box::new(tcp)
                }
            };
            if secure {
                self.tls_over(&bare, io).await
            } else {
                Ok(io)
            }
        };
        let io = tokio::time::timeout(self.connect_timeout, connect)
            .await
            .map_err(|_| failed("the connection timed out".into()))??;
        let mut config = tungstenite::protocol::WebSocketConfig::default();
        config.max_message_size = Some(MAX_BODY);
        config.max_frame_size = Some(MAX_BODY);
        match tokio_tungstenite::client_async_with_config(request, io, Some(config)).await {
            Ok((ws, resp)) => Ok(Response {
                status: resp.status().as_u16(),
                headers: header_list(resp.headers()),
                body: Body::Socket(ws),
                rate_limit: None,
            }),
            Err(tungstenite::Error::Http(resp)) => {
                let status = resp.status().as_u16();
                let headers = header_list(resp.headers());
                let body = resp.into_body().unwrap_or_default();
                Ok(Response {
                    status,
                    headers,
                    body: Body::Bytes(body),
                    rate_limit: None,
                })
            }
            Err(e) => Err(failed(e.to_string())),
        }
    }

    /// A tunnel to `host:port` through `proxy` with `CONNECT`, Basic
    /// `Proxy-Authorization` from the proxy URL's user-info.
    async fn tunnel(&self, proxy: &Url, host: &str, port: u16) -> Result<Box<dyn Io>, Error> {
        use base64::Engine as _;
        use std::fmt::Write as _;
        let ph = proxy
            .host_str()
            .unwrap_or_default()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned();
        let pp = proxy.port_or_known_default().unwrap_or(80);
        let failed = |reason: String| Error::Connection {
            host: ph.clone(),
            reason: format!("the proxy: {reason}"),
        };
        let tcp = tokio::net::TcpStream::connect((ph.as_str(), pp))
            .await
            .map_err(|e| failed(e.to_string()))?;
        let mut io: Box<dyn Io> = if proxy.scheme() == "https" {
            self.tls_over(&ph, Box::new(tcp)).await?
        } else {
            Box::new(tcp)
        };
        let mut head = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n");
        if !proxy.username().is_empty() || proxy.password().is_some() {
            let decode = |s: &str| percent_decode(s);
            let pair = format!(
                "{}:{}",
                decode(proxy.username()),
                decode(proxy.password().unwrap_or_default())
            );
            let encoded = base64::engine::general_purpose::STANDARD.encode(pair.as_bytes());
            let _ = write!(head, "Proxy-Authorization: Basic {encoded}\r\n");
        }
        head.push_str("\r\n");
        io.write_all(head.as_bytes())
            .await
            .map_err(|e| failed(e.to_string()))?;
        let mut answer = Vec::new();
        let mut byte = [0u8; 1];
        while !answer.ends_with(b"\r\n\r\n") {
            if answer.len() > 8192 {
                return Err(failed("the CONNECT answer is too long".into()));
            }
            let n = io
                .read(&mut byte)
                .await
                .map_err(|e| failed(e.to_string()))?;
            if n == 0 {
                return Err(failed("closed during CONNECT".into()));
            }
            answer.push(byte[0]);
        }
        let line = String::from_utf8_lossy(&answer);
        let status = line.split_whitespace().nth(1).unwrap_or_default();
        if status != "200" {
            return Err(failed(format!("CONNECT answered {status}")));
        }
        Ok(io)
    }

    #[cfg(feature = "rustls")]
    async fn tls_over(&self, host: &str, io: Box<dyn Io>) -> Result<Box<dyn Io>, Error> {
        let mut config = (*self.tls).clone();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let name = rustls::pki_types::ServerName::try_from(host.to_owned())
            .map_err(|e| ConfigError::Http(e.to_string()))?;
        let stream = tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(name, io)
            .await
            .map_err(|e| Error::Connection {
                host: host.to_owned(),
                reason: e.to_string(),
            })?;
        Ok(Box::new(stream))
    }

    #[cfg(not(feature = "rustls"))]
    #[allow(clippy::unused_self, reason = "the same signature as with rustls")]
    fn tls_over(
        &self,
        _host: &str,
        _io: Box<dyn Io>,
    ) -> impl std::future::Future<Output = Result<Box<dyn Io>, Error>> + Send {
        std::future::ready(Err(
            ConfigError::Http("TLS needs the `rustls` feature".into()).into()
        ))
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn header_list(h: &tokio_tungstenite::tungstenite::http::HeaderMap) -> Headers {
    Headers::new(h.iter().map(|(k, v)| {
        (
            k.as_str().to_owned(),
            v.to_str().unwrap_or_default().to_owned(),
        )
    }))
}

/// A transport failure as the error family has it: a timeout, a request that could not
/// be built, or a connection that failed.
fn failed(host: &str, e: &reqwest::Error) -> Error {
    if e.is_timeout() {
        Error::Timeout {
            host: host.to_owned(),
            secs: 0,
        }
    } else if e.is_builder() {
        ConfigError::Http(transport_reason(e)).into()
    } else {
        Error::Connection {
            host: host.to_owned(),
            reason: transport_reason(e),
        }
    }
}

#[cfg(feature = "rustls")]
mod tls {
    //! The TLS configuration every connection of a client shares.

    use std::sync::Arc;

    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::pem::PemObject as _;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
    use rustls::{DigitallySignedStruct, SignatureScheme};

    use crate::config::Settings;
    use crate::error::{ConfigError, Problem};

    fn problem(setting: &str, message: String) -> ConfigError {
        ConfigError::Invalid {
            problems: vec![Problem {
                setting: setting.to_owned(),
                source: String::new(),
                message,
            }],
        }
    }

    fn read(setting: &str, path: &str) -> Result<Vec<u8>, ConfigError> {
        std::fs::read(path).map_err(|e| problem(setting, format!("cannot read {path}: {e}")))
    }

    /// The rustls configuration for `s`.
    pub(super) fn config(s: &Settings) -> Result<rustls::ClientConfig, ConfigError> {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let http = |e: &dyn std::fmt::Display| ConfigError::Http(e.to_string());
        let extra: Vec<CertificateDer<'static>> = match &s.ca_bundle {
            None => Vec::new(),
            Some(path) => {
                let pem = read("ca_bundle", path)?;
                let certs: Vec<_> = CertificateDer::pem_slice_iter(&pem)
                    .collect::<Result<_, _>>()
                    .map_err(|e| {
                        problem("ca_bundle", format!("{path} is not PEM certificates: {e}"))
                    })?;
                if certs.is_empty() {
                    return Err(problem("ca_bundle", format!("{path} holds no certificate")));
                }
                certs
            }
        };
        let verifier: Arc<dyn ServerCertVerifier> = if s.system_trust {
            #[cfg(not(target_os = "android"))]
            let v = if extra.is_empty() {
                rustls_platform_verifier::Verifier::new(Arc::clone(&provider))
            } else {
                rustls_platform_verifier::Verifier::new_with_extra_roots(
                    extra,
                    Arc::clone(&provider),
                )
            };
            #[cfg(target_os = "android")]
            let v = if extra.is_empty() {
                rustls_platform_verifier::Verifier::new(Arc::clone(&provider))
            } else {
                return Err(problem("ca_bundle", "is not supported on Android".into()));
            };
            Arc::new(v.map_err(|e| http(&e))?)
        } else {
            let mut roots = rustls::RootCertStore::empty();
            for c in extra {
                roots
                    .add(c)
                    .map_err(|e| problem("ca_bundle", e.to_string()))?;
            }
            rustls::client::WebPkiServerVerifier::builder_with_provider(
                Arc::new(roots),
                Arc::clone(&provider),
            )
            .build()
            .map_err(|e| http(&e))?
        };
        let verifier: Arc<dyn ServerCertVerifier> = if s.pinned_keys.is_empty() {
            verifier
        } else {
            Arc::new(Pinned::new(verifier, &s.pinned_keys)?)
        };
        let builder = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| http(&e))?
            .dangerous()
            .with_custom_certificate_verifier(verifier);
        let mut config = match (&s.client_cert, &s.client_key) {
            (Some(cert), Some(key)) => {
                let pem = read("client_cert", cert)?;
                let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&pem)
                    .collect::<Result<_, _>>()
                    .map_err(|e| problem("client_cert", format!("{cert} is not PEM: {e}")))?;
                let key = private_key(key, s)?;
                builder
                    .with_client_auth_cert(chain, key)
                    .map_err(|e| problem("client_key", e.to_string()))?
            }
            _ => builder.with_no_client_auth(),
        };
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Ok(config)
    }

    fn private_key(path: &str, s: &Settings) -> Result<PrivateKeyDer<'static>, ConfigError> {
        let pem = read("client_key", path)?;
        if let Some(password) = &s.client_key_password {
            return encrypted(&pem, password.expose(), path);
        }
        PrivateKeyDer::from_pem_slice(&pem).map_err(|e| {
            problem(
                "client_key",
                format!("{path} is not a PEM private key (an encrypted one needs client_key_password): {e}"),
            )
        })
    }

    #[cfg(feature = "encrypted-key")]
    fn encrypted(
        pem: &[u8],
        password: &str,
        path: &str,
    ) -> Result<PrivateKeyDer<'static>, ConfigError> {
        let text = std::str::from_utf8(pem)
            .map_err(|_| problem("client_key", format!("{path} is not PEM")))?;
        let (_, doc) = pkcs8::SecretDocument::from_pem(text).map_err(|e| {
            problem(
                "client_key",
                format!("{path} is not an encrypted PKCS#8 key: {e}"),
            )
        })?;
        let info = pkcs8::EncryptedPrivateKeyInfo::try_from(doc.as_bytes()).map_err(|e| {
            problem(
                "client_key",
                format!("{path} is not an encrypted PKCS#8 key: {e}"),
            )
        })?;
        let plain = info
            .decrypt(password)
            .map_err(|_| problem("client_key_password", format!("does not decrypt {path}")))?;
        Ok(PrivateKeyDer::Pkcs8(plain.as_bytes().to_vec().into()))
    }

    #[cfg(not(feature = "encrypted-key"))]
    fn encrypted(_: &[u8], _: &str, path: &str) -> Result<PrivateKeyDer<'static>, ConfigError> {
        Err(problem(
            "client_key_password",
            format!("decrypting {path} needs the crate's `encrypted-key` feature"),
        ))
    }

    /// The system's verifier, then the end-entity key must be one of the pins (SR-06).
    #[derive(Debug)]
    struct Pinned {
        inner: Arc<dyn ServerCertVerifier>,
        pins: Vec<Vec<u8>>,
    }

    impl Pinned {
        fn new(inner: Arc<dyn ServerCertVerifier>, pins: &[String]) -> Result<Self, ConfigError> {
            use base64::Engine as _;
            let pins = pins
                .iter()
                .map(|p| {
                    base64::engine::general_purpose::STANDARD
                        .decode(p)
                        .map_err(|_| problem("pinned_keys", format!("{p:?} is not base64")))
                })
                .collect::<Result<_, _>>()?;
            Ok(Self { inner, pins })
        }
    }

    fn sha256(bytes: &[u8]) -> Vec<u8> {
        match rustls::crypto::aws_lc_rs::cipher_suite::TLS13_AES_128_GCM_SHA256 {
            rustls::SupportedCipherSuite::Tls13(cs) => {
                cs.common.hash_provider.hash(bytes).as_ref().to_vec()
            }
            rustls::SupportedCipherSuite::Tls12(cs) => {
                cs.common.hash_provider.hash(bytes).as_ref().to_vec()
            }
        }
    }

    impl ServerCertVerifier for Pinned {
        fn verify_server_cert(
            &self,
            end_entity: &CertificateDer<'_>,
            intermediates: &[CertificateDer<'_>],
            server_name: &ServerName<'_>,
            ocsp_response: &[u8],
            now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            let ok = self.inner.verify_server_cert(
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                now,
            )?;
            let cert = webpki::EndEntityCert::try_from(end_entity)
                .map_err(|e| rustls::Error::General(e.to_string()))?;
            let hash = sha256(cert.subject_public_key_info().as_ref());
            if self.pins.contains(&hash) {
                Ok(ok)
            } else {
                Err(rustls::Error::General(
                    "the server's public key is not one of the pinned keys".into(),
                ))
            }
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            self.inner.verify_tls12_signature(message, cert, dss)
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            self.inner.verify_tls13_signature(message, cert, dss)
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.inner.supported_verify_schemes()
        }
    }
}
