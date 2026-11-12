//! `iohr ui`: serve the InOrbit web UI (`sdk/ui`) on a loopback address (RFC 0217).
//!
//! The listener is the sign-in redirect's (`iohr_auth::loopback`): 127.0.0.1 first, else
//! `[::1]`, a read window and a head cap per connection. It adds what a file server
//! needs and no HTTP stack: GET and HEAD only, a `Host` that names this listener (so a
//! rebound DNS name cannot reach it), paths that cannot leave the UI folder or follow a
//! link, `/api/*` only from this listener's own pages (`Sec-Fetch-Site` and `Origin`), a
//! capped number of connections, and the same strict headers on every answer.
//!
//! Reaching the page takes the per-launch secret (`session`, RFC 0217 round 4): `iohr ui
//! open` (or `serve --open`) puts it in the first address as `?launch=`, the server answers
//! `303` to `/` and sets an `HttpOnly`, `SameSite=Strict` cookie, and every other answer
//! needs that cookie. The page never holds a credential: `GET /api/session`,
//! `POST /api/session/login` and `/logout` and `GET /api/proxy/v1/...` act as the profile
//! `iohr login` signed in, from the same store.
//!
//! `build`, `dev` and `check` run the UI package's own scripts through pnpm (the runner
//! `iohr sdk add` uses: no shell, the exit code passed on), so a developer, mise and CI
//! call one thing. A running `serve` records its address in the data folder
//! (`ui/serving.json`, 0600) for `iohr ui open`, and removes it when it stops.
//!
//! `GET /api/extensions` lists the installed extensions that serve a panel
//! (`ext::install::Store::panels`, the store `iohr ext install` writes), with the port of
//! each one this serve has started. `POST /api/extensions/<name>/start` and `.../stop`
//! start and stop one (RFC 0217, question 3) through `ext::run::start_panel`, the one
//! launch path `iohr ext run` uses, with its full trust check. Only the UI's own page can
//! call them: `Sec-Fetch-Site: same-origin`, an `Origin` that is exactly this listener's,
//! and the per-serve token the page carries in `<meta name="iohr-token">` (sent in the
//! `x-iohr-token` header, compared in constant time), with no body and nothing else from
//! the client: the name must equal an installed panel's and builds no path. One instance
//! per extension; a panel that names the UI's port or another panel's is stopped. Every
//! page answer allows framing exactly the ports this serve started (`frame-src`), and the
//! UI itself is never framed. When the serve stops, every panel it started is stopped
//! (SIGTERM to its process group, SIGKILL after 5 s).

mod forward;
#[cfg(test)]
mod forward_tests;
mod live;
#[cfg(test)]
mod live_tests;
mod session;
#[cfg(test)]
mod session_tests;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, Read as _};
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use iohr_auth::loopback::{MAX_HEAD, READ_WINDOW, bind_loopback};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::time::timeout;

use crate::Env;
use crate::cli::{Global, UiArgs, UiCommand, UiPackage, UiServe};
use crate::commands::sdk_add::{resolve, run_program};
use crate::commands::session::data_dir;
use crate::error::Error;
use crate::ext::install::Store;
use crate::ext::run::{PANEL_LINE_WINDOW, PANEL_STOP_GRACE, StartedPanel, start_panel};
use crate::output::Out;
use crate::ui::Ui;

/// Connections answered at once; one more is closed unanswered.
const MAX_CONNECTIONS: usize = 16;
/// How long one answer may take to write.
const WRITE_WINDOW: Duration = Duration::from_secs(30);
/// The largest file served; a built UI's biggest chunk is far smaller.
const MAX_FILE: u64 = 32 * 1024 * 1024;
/// The headers every answer carries besides its policy and its caching.
const SECURITY_HEADERS: &str = "Referrer-Policy: no-referrer\r\n\
     X-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\n\
     Cross-Origin-Opener-Policy: same-origin\r\n\
     Cross-Origin-Resource-Policy: same-origin\r\n";
/// Every answer's policy: the page's own scripts, styles and images, nothing framed
/// around it, no plugins, no forms posting elsewhere.
const CSP: &str = "default-src 'self'; img-src 'self' data:; object-src 'none'; \
                   base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// The file a running `serve` records its address in, below the data folder's `ui/`.
const SERVING: &str = "serving.json";
/// The largest `serving.json` read; it holds one address and one path.
const MAX_SERVING: u64 = 16 * 1024;
/// How long `open` waits to see the recorded server answer.
const ALIVE_WINDOW: Duration = Duration::from_secs(1);

/// `iohr ui [verb]`.
pub(crate) async fn run(g: &Global, env: &Env, args: UiArgs, out: Out) -> Result<(), Error> {
    match args.command {
        None => {
            let serve = UiServe {
                open: true,
                ..UiServe::default()
            };
            serve_until_stopped(g, env, &serve, out).await
        }
        Some(UiCommand::Serve(serve)) => serve_until_stopped(g, env, &serve, out).await,
        Some(UiCommand::Build(package)) => script(&package, "build", out),
        Some(UiCommand::Dev(package)) => script(&package, "dev", out),
        Some(UiCommand::Check(package)) => script(&package, "check", out),
        Some(UiCommand::Open) => open(g, out).await,
    }
}

fn working_folder() -> Result<PathBuf, Error> {
    std::env::current_dir().map_err(|e| Error::Failed(format!("the working folder: {}", e.kind())))
}

/// Runs the UI package's script `name` with pnpm and passes its exit code on.
fn script(args: &UiPackage, name: &str, out: Out) -> Result<(), Error> {
    let dir = ui_package(args.package.as_deref(), &working_folder()?)?;
    let pnpm = resolve(Path::new("pnpm")).ok_or_else(|| {
        Error::Usage(
            "pnpm is not on PATH: install it (`corepack enable pnpm`), then run this again".into(),
        )
    })?;
    Out::note(&format!("Running `pnpm run {name}` in {}", dir.display()));
    run_program(
        &pnpm,
        &["run".to_owned(), name.to_owned()],
        &dir,
        "pnpm",
        out.json,
    )?;
    if out.json {
        Out::print_json(&serde_json::json!({ "script": name, "dir": dir.display().to_string() }));
    }
    Ok(())
}

/// The UI package folder: `package` when given (it must hold `package.json`), else the
/// nearest `ui/` with a `package.json` at or above `from`.
fn ui_package(package: Option<&Path>, from: &Path) -> Result<PathBuf, Error> {
    if let Some(dir) = package {
        if dir.join("package.json").is_file() {
            return canonical(dir);
        }
        return Err(Error::Usage(format!(
            "{} holds no package.json; point --package at the UI package",
            dir.display()
        )));
    }
    nearest(from, &["ui"], "package.json").ok_or_else(|| {
        Error::Usage("no UI package here: run this in the sdk checkout, or pass --package".into())
    })
}

/// The nearest `<ancestor>/<sub...>` at or above `from` that holds the file `marker`,
/// canonical.
fn nearest(from: &Path, sub: &[&str], marker: &str) -> Option<PathBuf> {
    from.ancestors().find_map(|folder| {
        let dir = sub.iter().fold(folder.to_path_buf(), |d, s| d.join(s));
        if dir.join(marker).is_file() {
            std::fs::canonicalize(&dir).ok()
        } else {
            None
        }
    })
}

fn canonical(dir: &Path) -> Result<PathBuf, Error> {
    std::fs::canonicalize(dir)
        .map_err(|e| Error::Failed(format!("{}: {}", dir.display(), e.kind())))
}

/// Opens the UI the running `serve` recorded, when it is a loopback address that answers.
async fn open(g: &Global, out: Out) -> Result<(), Error> {
    let folder = data_dir(g)?.join("ui");
    let path = folder.join(SERVING);
    let file = crate::runner::private::open_trusted(&path, false)
        .map_err(Error::Failed)?
        .ok_or_else(|| Error::Usage("nothing is being served: run `iohr ui serve` first".into()))?;
    let mut text = String::new();
    file.take(MAX_SERVING)
        .read_to_string(&mut text)
        .map_err(|e| Error::Failed(format!("{}: {}", path.display(), e.kind())))?;
    let launch = recorded_launch(&text);
    let Some((url, addr)) = recorded_url(&text) else {
        return Err(Error::Failed(format!(
            "{} does not hold a loopback address; refused",
            path.display()
        )));
    };
    if !matches!(
        timeout(ALIVE_WINDOW, TcpStream::connect(addr)).await,
        Ok(Ok(_))
    ) {
        forget(&folder, &url);
        return Err(Error::Usage(
            "the recorded `iohr ui serve` is no longer running: start it again".into(),
        ));
    }
    // The first address carries the launch secret, so the browser is signed in; the
    // secret is printed only when no browser can be opened and the person asked for it.
    let first = launch.map_or_else(|| url.clone(), |l| format!("{url}?launch={l}"));
    if out.json {
        Out::print_json(&serde_json::json!({ "url": url }));
    }
    if !crate::browser::open(&first) {
        if !out.json {
            Out::raw(format!("{first}\n").as_bytes());
        }
        Out::note("Open that address in a browser on this machine; it signs the browser in.");
    }
    Ok(())
}

/// The `url` in a recorded `serving.json`, and its address, when it is exactly
/// `http://127.0.0.1:<port>/` or `http://[::1]:<port>/` with a port; anything else is
/// `None`, so `open` never hands the browser another address.
fn recorded_url(text: &str) -> Option<(String, SocketAddr)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let url = value.get("url")?.as_str()?;
    let addr: SocketAddr = url
        .strip_prefix("http://")?
        .strip_suffix('/')?
        .parse()
        .ok()?;
    let own =
        addr.ip().is_loopback() && addr.port() != 0 && format!("http://{}/", host_of(addr)) == url;
    own.then(|| (url.to_owned(), addr))
}

/// The launch secret in a recorded `serving.json`: 64 hex digits, else none.
fn recorded_launch(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let launch = value.get("launch")?.as_str()?;
    (launch.len() == 64 && launch.bytes().all(|b| b.is_ascii_hexdigit())).then(|| launch.to_owned())
}

/// Records `url` (served from `root`) and the launch secret for `iohr ui open`; the file
/// is private to the person (0600).
fn remember(folder: &Path, url: &str, root: &Path, launch: &str) -> io::Result<()> {
    let body = serde_json::json!({
        "url": url,
        "dir": root.display().to_string(),
        "pid": std::process::id(),
        "launch": launch,
    });
    iohr_auth::write_private(folder, SERVING, body.to_string().as_bytes())
}

/// Removes the record when it still names `url`: a later `serve` may have replaced it.
fn forget(folder: &Path, url: &str) {
    let path = folder.join(SERVING);
    let ours = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| recorded_url(&t))
        .is_some_and(|(u, _)| u == url);
    if ours {
        let _ = std::fs::remove_file(path);
    }
}

async fn serve_until_stopped(g: &Global, env: &Env, args: &UiServe, out: Out) -> Result<(), Error> {
    let root = ui_root(args.dir.as_deref(), &working_folder()?)?;
    let port = args.port.unwrap_or(0);
    let panels = Panels::new(g.clone(), env.clone())
        .map_err(|e| Error::Failed(format!("the start token: {}", e.kind())))?;
    let server = Server::bind(&root, port, super::ext::store(g)?, panels)
        .await
        .map_err(|e| listen_error(port, &e))?;
    let url = server.url();
    if out.json {
        Out::print_json(&serde_json::json!({ "url": url, "dir": root.display().to_string() }));
    } else {
        let ui = Ui::detect(false);
        ui.intro("iohr ui");
        ui.info(&format!("Serving {} at {url}", root.display()));
        ui.info("Ctrl-C stops it.");
    }
    let folder = data_dir(g).map(|d| d.join("ui"));
    let recorded = match &folder {
        Ok(f) => remember(f, &url, &root, &server.launch())
            .map(|()| server.keep_record(f.clone(), url.clone(), root.clone()))
            .map_err(|e| e.kind().to_string()),
        Err(e) => Err(e.to_string()),
    };
    if let Err(why) = &recorded {
        Out::note(&format!(
            "`iohr ui open` will not find this server ({why})."
        ));
    }
    if args.open && !crate::browser::open(&format!("{url}?launch={}", server.launch())) {
        Out::note("Run `iohr ui open` to open it in a browser, signed in.");
    }
    server
        .serve(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    if let (Ok(folder), Ok(())) = (&folder, &recorded) {
        forget(folder, &url);
    }
    Ok(())
}

/// What a failed bind tells the person: a taken `--port` is theirs to change.
fn listen_error(port: u16, e: &io::Error) -> Error {
    if e.kind() == io::ErrorKind::AddrInUse {
        return Error::Usage(format!(
            "port {port} is already in use on loopback: pass another --port, or none to let \
             the system pick"
        ));
    }
    Error::Failed(format!("listen on loopback: {}", e.kind()))
}

/// The built UI folder: `dir` when given (it must hold `index.html`), else the nearest
/// `ui/dist` with an `index.html` at or above `from`.
fn ui_root(dir: Option<&Path>, from: &Path) -> Result<PathBuf, Error> {
    if let Some(dir) = dir {
        if dir.join("index.html").is_file() {
            return canonical(dir);
        }
        return Err(Error::Usage(format!(
            "{} holds no index.html; point --dir (IOHR_UI_DIR) at a built UI",
            dir.display()
        )));
    }
    nearest(from, &["ui", "dist"], "index.html").ok_or_else(|| {
        Error::Usage(
            "no built UI here: run `iohr ui build` in the sdk checkout, or pass --dir".into(),
        )
    })
}

/// The UI listener: a loopback socket and what every answer reads.
pub(crate) struct Server {
    listener: TcpListener,
    site: Arc<Site>,
    addr: SocketAddr,
}

/// What every answer reads: the canonical UI folder, the extension store it lists, the
/// panels this serve started, and the listener's own `Host`.
struct Site {
    root: PathBuf,
    store: Arc<Store>,
    panels: Arc<Panels>,
    access: Arc<session::Access>,
    host: String,
    port: u16,
}

impl Server {
    /// Binds `port` (0: the system picks) on loopback, serving the canonical `root`,
    /// listing the panels in `store` and starting them through `panels`.
    pub(crate) async fn bind(
        root: &Path,
        port: u16,
        store: Store,
        panels: Panels,
    ) -> io::Result<Self> {
        Self::bind_with(root, port, store, panels, forward::Limits::default()).await
    }

    /// [`bind`](Self::bind) with the proxy's bounds set (tests make them small).
    async fn bind_with(
        root: &Path,
        port: u16,
        store: Store,
        panels: Panels,
        limits: forward::Limits,
    ) -> io::Result<Self> {
        let root = std::fs::canonicalize(root)?;
        let listener = bind_loopback(port).await?;
        let addr = listener.local_addr()?;
        let access = session::Access::new(
            panels.global.clone(),
            panels.env.clone(),
            addr.port(),
            limits,
        )?;
        let site = Arc::new(Site {
            root,
            store: Arc::new(store),
            panels: Arc::new(panels),
            access: Arc::new(access),
            host: host_of(addr),
            port: addr.port(),
        });
        Ok(Self {
            listener,
            site,
            addr,
        })
    }

    /// The address a browser opens.
    pub(crate) fn url(&self) -> String {
        format!("http://{}/", host_of(self.addr))
    }

    /// The per-launch secret that signs a browser in (`?launch=`); recorded for `iohr ui
    /// open`, never printed by `serve`.
    pub(crate) fn launch(&self) -> String {
        self.site.access.launch()
    }

    /// Records where `iohr ui open` reads the launch secret, so the one that replaces a
    /// spent secret is written there.
    pub(crate) fn keep_record(&self, folder: PathBuf, url: String, root: PathBuf) {
        self.site.access.keep_record(folder, url, root);
    }

    /// Answers connections until `stop` finishes, then stops every panel it started.
    pub(crate) async fn serve(self, stop: impl Future<Output = ()>) {
        let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        tokio::pin!(stop);
        loop {
            tokio::select! {
                () = &mut stop => break,
                accepted = self.listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue };
                    // Over the cap: the connection closes unanswered.
                    let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else { continue };
                    let site = Arc::clone(&self.site);
                    tokio::spawn(async move {
                        answer(stream, &site).await;
                        drop(permit);
                    });
                }
            }
        }
        self.site.access.close();
        self.site.panels.stop_all().await;
    }
}

/// `127.0.0.1:<port>` or `[::1]:<port>`, as a `Host` header names it.
fn host_of(addr: SocketAddr) -> String {
    match addr {
        SocketAddr::V4(a) => format!("{}:{}", a.ip(), a.port()),
        SocketAddr::V6(a) => format!("[{}]:{}", a.ip(), a.port()),
    }
}

async fn answer(mut stream: TcpStream, site: &Site) {
    let Ok(Ok((head, rest))) = timeout(READ_WINDOW, read_request_head(&mut stream)).await else {
        return;
    };
    match reply_to(&head, site).await {
        Next::Reply(reply) => {
            let _ = timeout(WRITE_WINDOW, reply.write(&mut stream)).await;
        }
        Next::Proxy(request) => {
            forward::forward(&mut stream, &request, rest, site.access.as_ref()).await;
        }
        Next::Live(request) => {
            live::serve(stream, &request, &rest, &site.panels, site.access.as_ref()).await;
        }
    }
}

/// A request head, read up to the blank line that ends it (or [`MAX_HEAD`] bytes), and the
/// bytes of the body that came with it. The head is capped on its own, so body bytes in the
/// same read are never cut off with it.
async fn read_request_head(stream: &mut TcpStream) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let mut all = Vec::with_capacity(1024);
    let mut buf = [0_u8; 1024];
    loop {
        if let Some(end) = all.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = all.split_off(end + 4);
            return Ok((all, rest));
        }
        if all.len() >= MAX_HEAD {
            all.truncate(MAX_HEAD);
            return Ok((all, Vec::new()));
        }
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Ok((all, Vec::new()));
        }
        all.extend_from_slice(&buf[..n]);
    }
}

/// What the gates decided for one request head: an answer to write, or a call of the API
/// for the page, which reads its body and may stream its answer (`session::forward`).
enum Next {
    Reply(Reply),
    Proxy(Request),
    /// `GET /api/live` through the gates: the WebSocket upgrade (`live`).
    Live(Request),
}

/// One answer, written whole.
#[derive(Debug, PartialEq, Eq)]
struct Reply {
    status: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
    /// `HEAD`: the headers say the length, no body follows.
    head_only: bool,
    /// `index.html` and errors are never cached; hashed assets may be.
    no_store: bool,
    allow: bool,
    /// The loopback ports a page answer may frame: the panels this serve started.
    frame_src: Vec<u16>,
    /// Further header lines, whole and without their line ending (`Location`, `Set-Cookie`).
    headers: Vec<String>,
}

impl Reply {
    fn error(status: &'static str) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            body: format!("{status}\n").into_bytes(),
            head_only: false,
            no_store: true,
            allow: false,
            frame_src: Vec::new(),
            headers: Vec::new(),
        }
    }

    /// An error with a JSON body naming it: `{"error": {"status": ..., "message": ...}}`.
    fn problem(status: &'static str, message: &str) -> Self {
        let body = serde_json::json!({ "error": { "status": status, "message": message } });
        Self {
            content_type: "application/json",
            body: body.to_string().into_bytes(),
            ..Self::error(status)
        }
    }

    /// `200 OK` with a JSON body, never cached.
    fn json(value: &serde_json::Value) -> Self {
        let Ok(body) = serde_json::to_vec(value) else {
            return Self::error("500 Internal Server Error");
        };
        Self {
            status: "200 OK",
            content_type: "application/json",
            body,
            ..Self::error("200 OK")
        }
    }

    async fn write(&self, stream: &mut TcpStream) -> io::Result<()> {
        let cache = if self.no_store {
            "no-store"
        } else {
            "no-cache"
        };
        let allow = if self.allow {
            "Allow: GET, HEAD\r\n"
        } else {
            ""
        };
        let mut csp = CSP.to_owned();
        if !self.frame_src.is_empty() {
            csp.push_str("; frame-src");
            for port in &self.frame_src {
                let _ = write!(csp, " http://127.0.0.1:{port}");
            }
        }
        let extra = self.headers.iter().fold(String::new(), |mut all, h| {
            let _ = write!(all, "{h}\r\n");
            all
        });
        let head = format!(
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{allow}{extra}\
             Content-Security-Policy: {csp}\r\nCache-Control: {cache}\r\n\
             {SECURITY_HEADERS}Connection: close\r\n\r\n",
            self.status,
            self.content_type,
            self.body.len()
        );
        stream.write_all(head.as_bytes()).await?;
        if !self.head_only {
            stream.write_all(&self.body).await?;
        }
        stream.shutdown().await
    }
}

/// The answer to one request head, or the call of the API it asks for.
async fn reply_to(head: &[u8], site: &Site) -> Next {
    let host = site.host.as_str();
    let request = match parse(head) {
        Ok(r) => r,
        Err(status) => return Next::Reply(Reply::error(status)),
    };
    if !host_allowed(request.host.as_deref(), host) {
        return Next::Reply(Reply::error("403 Forbidden"));
    }
    // The session cookie, or the launch secret that is exchanged for it (`session`).
    match site.access.entry(&request) {
        session::Entry::Open => {}
        session::Entry::Exchange if matches!(request.method.as_str(), "GET" | "HEAD") => {
            return Next::Reply(site.access.exchange());
        }
        _ => return Next::Reply(unsigned(&request)),
    }
    // A call of the API for the page: a read, a write or a stream, `/api/proxy/v1/...`.
    // Reads need the page's own origin and the token; a write needs the stricter
    // `post_caller_allowed` (a same-origin fetch, the exact `Origin`, the token).
    if let Some(next) = proxied(&request, site) {
        return next;
    }
    if decoded_path(&request.target).is_some_and(|p| p == "/api/live") {
        return Next::live_gate(&request, site);
    }
    Next::Reply(reply_to_page(&request, site).await)
}

impl Next {
    /// `/api/live`: a `GET` upgrade from the page's own origin (the token is the first
    /// frame, `live`), else the refusal.
    fn live_gate(request: &Request, site: &Site) -> Self {
        if request.method != "GET" {
            return Self::Reply(Reply {
                allow: true,
                ..Reply::error("405 Method Not Allowed")
            });
        }
        if !live::upgrade_allowed(request, &site.host) {
            return Self::Reply(Reply::error("403 Forbidden"));
        }
        Self::Live(request.clone())
    }
}

/// A call of `/api/proxy/...`: through the gates, or the refusal.
fn proxied(request: &Request, site: &Site) -> Option<Next> {
    let path = decoded_path(&request.target)?;
    if !path.starts_with("/api/proxy/") {
        return None;
    }
    let write = matches!(request.method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE");
    let read = matches!(request.method.as_str(), "GET" | "HEAD");
    if !write && !read {
        return Some(Next::Reply(Reply {
            allow: true,
            ..Reply::error("405 Method Not Allowed")
        }));
    }
    let allowed = if write {
        post_caller_allowed(request, &site.host, &site.panels)
    } else {
        api_caller_allowed(request, &site.host)
            && site.panels.token_matches(request.token.as_deref())
    };
    Some(if allowed {
        Next::Proxy(Request {
            target: request.target.clone(),
            method: request.method.clone(),
            host: request.host.clone(),
            origin: request.origin.clone(),
            fetch_site: request.fetch_site.clone(),
            token: None,
            cookie: None,
            content_length: request.content_length.clone(),
            transfer_encoding: request.transfer_encoding.clone(),
            content_type: request.content_type.clone(),
            accept: request.accept.clone(),
            ..Request::default()
        })
    } else {
        Next::Reply(Reply::error("403 Forbidden"))
    })
}

/// The answer to a request that is not a call of the API: the page, `/api/session`,
/// `/api/extensions`, a start or a stop.
async fn reply_to_page(request: &Request, site: &Site) -> Reply {
    let host = site.host.as_str();
    let head_only = match request.method.as_str() {
        "GET" => false,
        "HEAD" => true,
        "POST" if is_api(&request.target) => return post(request, site).await,
        _ => {
            return Reply {
                allow: true,
                ..Reply::error("405 Method Not Allowed")
            };
        }
    };
    if is_api(&request.target) {
        if !api_caller_allowed(request, host) {
            return Reply::error("403 Forbidden");
        }
        // Any other `/api` path is 404, never the app page.
        let path = decoded_path(&request.target);
        let path = path.as_deref();
        let own_script = site.panels.token_matches(request.token.as_deref());
        let mut reply = match path {
            Some("/api/extensions") => extensions(Arc::clone(&site.store), &site.panels).await,
            Some("/api/session") if own_script => site.access.state().await,
            Some("/api/session") => Reply::error("403 Forbidden"),
            _ => Reply::error("404 Not Found"),
        };
        reply.head_only = head_only;
        return reply;
    }
    let mut reply = match file_for(&site.root, &request.target) {
        Ok(path) => serve_file(&path).await,
        Err(status) => Reply::error(status),
    };
    if reply.status == "200 OK" && reply.content_type.starts_with("text/html") {
        reply.body = with_token(&reply.body, &site.panels.token);
        reply.frame_src = site.panels.ports().into_values().collect();
    }
    reply.head_only = head_only;
    reply
}

/// The answer to a request that carries neither the session cookie nor the launch secret.
fn unsigned(request: &Request) -> Reply {
    if is_api(&request.target) {
        return Reply::problem(
            "401 Unauthorized",
            "this browser is not signed in to iohr ui: open it with `iohr ui open`",
        );
    }
    Reply {
        content_type: "text/plain; charset=utf-8",
        body: b"401 Unauthorized\nOpen the UI with `iohr ui open` (it signs this browser in).\n"
            .to_vec(),
        ..Reply::error("401 Unauthorized")
    }
}

/// A page with `<meta name="iohr-token" content="...">` right after its `<head>` (else
/// after its doctype, else first), so the UI's own script can send the start token.
fn with_token(page: &[u8], token: &str) -> Vec<u8> {
    let meta = format!("<meta name=\"iohr-token\" content=\"{token}\">");
    let find = |needle: &[u8]| {
        page.windows(needle.len())
            .position(|w| w.eq_ignore_ascii_case(needle))
    };
    let at = find(b"<head>").map(|i| i + b"<head>".len()).or_else(|| {
        page.get(..9)
            .is_some_and(|d| d.eq_ignore_ascii_case(b"<!doctype"))
            .then(|| page.iter().position(|b| *b == b'>').map(|i| i + 1))
            .flatten()
    });
    let at = at.unwrap_or(0);
    let mut out = Vec::with_capacity(page.len() + meta.len());
    out.extend_from_slice(&page[..at]);
    out.extend_from_slice(meta.as_bytes());
    out.extend_from_slice(&page[at..]);
    out
}

/// What a POST asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Start,
    Stop,
}

/// `POST /api/extensions/<name>/start` or `.../stop`, from the UI's own page only (see
/// [`post_caller_allowed`]), with no body. The target is matched raw, with no decoding
/// and no query, and `<name>` must equal an installed panel's name (or, to stop, a
/// running one's). Every other `/api` POST is 404.
async fn post(request: &Request, site: &Site) -> Reply {
    if !post_caller_allowed(request, &site.host, &site.panels) {
        return Reply::error("403 Forbidden");
    }
    let has_body = request.transfer_encoding.is_some()
        || request.content_length.as_deref().is_some_and(|l| l != "0");
    if has_body {
        return Reply::error("400 Bad Request");
    }
    match request.target.as_str() {
        "/api/session/login" => return site.access.login(),
        "/api/session/logout" => return site.access.logout().await,
        _ => {}
    }
    let Some(rest) = request.target.strip_prefix("/api/extensions/") else {
        return Reply::error("404 Not Found");
    };
    let (asked, action) = if let Some(n) = rest.strip_suffix("/start") {
        (n, Action::Start)
    } else if let Some(n) = rest.strip_suffix("/stop") {
        (n, Action::Stop)
    } else {
        return Reply::error("404 Not Found");
    };
    if action == Action::Stop && site.panels.ports().contains_key(asked) {
        return site.panels.stop(asked).await;
    }
    let store = Arc::clone(&site.store);
    let Ok(Ok(listed)) = tokio::task::spawn_blocking(move || store.panels()).await else {
        return Reply::error("500 Internal Server Error");
    };
    // The installed entry's own name goes on, never the client's text.
    let Some(name) = listed.into_iter().map(|p| p.name).find(|n| n == asked) else {
        return Reply::error("404 Not Found");
    };
    match action {
        Action::Start => {
            Arc::clone(&site.panels)
                .start(Arc::clone(&site.store), name, site.port)
                .await
        }
        Action::Stop => site.panels.stop(&name).await,
    }
}

/// A start or stop caller is the UI's own page: `Sec-Fetch-Site` is `same-origin`,
/// `Origin` is exactly this listener's (`http://<host>`), and `x-iohr-token` is this
/// serve's token. A missing header is a refusal.
fn post_caller_allowed(request: &Request, own: &str, panels: &Panels) -> bool {
    let site_ok = request
        .fetch_site
        .as_deref()
        .is_some_and(|site| site.eq_ignore_ascii_case("same-origin"));
    let origin_ok = request
        .origin
        .as_deref()
        .and_then(|o| o.strip_prefix("http://"))
        .is_some_and(|h| h == own);
    let token_ok = panels.token_matches(request.token.as_deref());
    site_ok && origin_ok && token_ok
}

/// What a started panel's slot holds.
enum Slot {
    /// `start_panel` is running; a second start or a stop is refused until it ends.
    Starting,
    /// Started and checked; `generation` tells this run from a later one of the same name.
    Running {
        generation: u64,
        panel: StartedPanel,
    },
}

/// The panels this serve started, one per extension, and what starting one needs: the
/// person's flags and environment (for the token channel), and the per-serve token.
pub(crate) struct Panels {
    global: Global,
    env: Env,
    token: String,
    closing: AtomicBool,
    generation: AtomicU64,
    slots: Mutex<BTreeMap<String, Slot>>,
}

/// What a finished start leaves.
enum Settled {
    Failed(Error),
    /// It names a port the UI or another panel holds; stopped by the caller.
    Taken(StartedPanel),
    Running {
        port: u16,
        generation: u64,
    },
}

impl Panels {
    /// No panel yet, and a fresh 256-bit token from the system's random source.
    pub(crate) fn new(global: Global, env: Env) -> io::Result<Self> {
        Ok(Self {
            global,
            env,
            token: random_hex()?,
            closing: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            slots: Mutex::new(BTreeMap::new()),
        })
    }

    fn slots(&self) -> MutexGuard<'_, BTreeMap<String, Slot>> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The port of every panel running now, by name.
    fn ports(&self) -> BTreeMap<String, u16> {
        self.slots()
            .iter()
            .filter_map(|(name, slot)| match slot {
                Slot::Running { panel, .. } => Some((name.clone(), panel.port)),
                Slot::Starting => None,
            })
            .collect()
    }

    /// Whether `given` is this serve's token, compared in constant time over its length.
    fn token_matches(&self, given: Option<&str>) -> bool {
        given.is_some_and(|g| same_bytes(g.as_bytes(), self.token.as_bytes()))
    }

    /// Starts `name` once: a running one answers its port, one still starting is 409.
    /// A start that fails, or names the UI's port or another panel's, is 502 with no
    /// detail on the wire; the reason goes to this terminal.
    async fn start(self: Arc<Self>, store: Arc<Store>, name: String, ui_port: u16) -> Reply {
        if self.closing.load(Ordering::SeqCst) {
            return Reply::error("503 Service Unavailable");
        }
        {
            let mut slots = self.slots();
            match slots.get(&name) {
                Some(Slot::Running { panel, .. }) => {
                    return Reply::json(&serde_json::json!({ "port": panel.port }));
                }
                Some(Slot::Starting) => return Reply::error("409 Conflict"),
                None => {
                    slots.insert(name.clone(), Slot::Starting);
                }
            }
        }
        let started = start_panel(&self.global, &self.env, &store, &name).await;
        let exited = started.as_ref().ok().map(StartedPanel::exited);
        match self.settle(&name, started, ui_port) {
            Settled::Failed(e) => {
                Out::note(&format!("The {name} panel did not start: {e}"));
                Reply::error("502 Bad Gateway")
            }
            Settled::Taken(panel) => {
                Out::note(&format!(
                    "The {name} panel was stopped: port {} is the UI's or another panel's",
                    panel.port
                ));
                panel.stop().await;
                Reply::error("502 Bad Gateway")
            }
            Settled::Running { port, generation } => {
                if let Some(exited) = exited {
                    let me = Arc::clone(&self);
                    tokio::spawn(async move {
                        exited.await;
                        me.ended(&name, generation).await;
                    });
                }
                Reply::json(&serde_json::json!({ "port": port }))
            }
        }
    }

    /// Records how a start ended. A panel finishing while the serve closes is still
    /// recorded, so [`Panels::stop_all`] stops it.
    fn settle(&self, name: &str, started: Result<StartedPanel, Error>, ui_port: u16) -> Settled {
        let mut slots = self.slots();
        slots.remove(name);
        let panel = match started {
            Err(e) => return Settled::Failed(e),
            Ok(panel) => panel,
        };
        let taken = panel.port == ui_port
            || slots.values().any(
                |s| matches!(s, Slot::Running { panel: other, .. } if other.port == panel.port),
            );
        if taken {
            return Settled::Taken(panel);
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst);
        let port = panel.port;
        slots.insert(name.to_owned(), Slot::Running { generation, panel });
        Settled::Running { port, generation }
    }

    /// A panel that exited by itself: its slot is freed and what is left of its group
    /// is stopped. A later run of the same name is left alone.
    async fn ended(&self, name: &str, generation: u64) {
        let panel = {
            let mut slots = self.slots();
            let ours = matches!(slots.get(name), Some(Slot::Running { generation: g, .. }) if *g == generation);
            if !ours {
                return;
            }
            match slots.remove(name) {
                Some(Slot::Running { panel, .. }) => panel,
                _ => return,
            }
        };
        panel.stop().await;
    }

    /// `{"stopped": true}` once `name`'s group is stopped, `false` when none ran; one
    /// still starting is 409.
    async fn stop(&self, name: &str) -> Reply {
        let panel = {
            let mut slots = self.slots();
            if matches!(slots.get(name), Some(Slot::Starting)) {
                return Reply::error("409 Conflict");
            }
            match slots.remove(name) {
                Some(Slot::Running { panel, .. }) => Some(panel),
                _ => None,
            }
        };
        let stopped = panel.is_some();
        if let Some(panel) = panel {
            panel.stop().await;
        }
        Reply::json(&serde_json::json!({ "stopped": stopped }))
    }

    /// Refuses new starts and stops every panel, all at once, waiting for starts in
    /// flight to finish (at most their first-line window and the stop grace).
    pub(crate) async fn stop_all(&self) {
        self.closing.store(true, Ordering::SeqCst);
        let deadline = tokio::time::Instant::now() + PANEL_LINE_WINDOW + PANEL_STOP_GRACE;
        loop {
            let (running, starting) = {
                let mut slots = self.slots();
                let all = std::mem::take(&mut *slots);
                let mut running = Vec::new();
                for (name, slot) in all {
                    match slot {
                        Slot::Running { panel, .. } => running.push(panel),
                        Slot::Starting => {
                            slots.insert(name, Slot::Starting);
                        }
                    }
                }
                (running, !slots.is_empty())
            };
            let stops: Vec<_> = running
                .into_iter()
                .map(|p| tokio::spawn(p.stop()))
                .collect();
            for stop in stops {
                let _ = stop.await;
            }
            if !starting || tokio::time::Instant::now() >= deadline {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// 256 bits from the system's random source, as 64 hex digits.
fn random_hex() -> io::Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| io::Error::other("no random source"))?;
    let mut text = String::with_capacity(64);
    for b in bytes {
        let _ = write!(text, "{b:02x}");
    }
    Ok(text)
}

/// Equal bytes, every byte compared whatever the first difference (lengths are public).
fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff = a
        .iter()
        .zip(b)
        .fold(0_u8, |acc, (x, y)| acc | std::hint::black_box(x ^ y));
    diff == 0
}

#[derive(Debug, Default, Clone)]
struct Request {
    method: String,
    target: String,
    host: Option<String>,
    origin: Option<String>,
    fetch_site: Option<String>,
    token: Option<String>,
    cookie: Option<String>,
    content_length: Option<String>,
    transfer_encoding: Option<String>,
    content_type: Option<String>,
    accept: Option<String>,
    upgrade: Option<String>,
    ws_key: Option<String>,
    ws_version: Option<String>,
}

/// The request line and the `Host`, `Origin`, `Sec-Fetch-Site`, `x-iohr-token`,
/// `Cookie`, `Content-Length`, `Transfer-Encoding`, `Content-Type` and `Accept` headers of a head; an error is the status to
/// answer. Any of them sent twice is refused.
fn parse(head: &[u8]) -> Result<Request, &'static str> {
    if head.len() >= MAX_HEAD && !head.windows(4).any(|w| w == b"\r\n\r\n") {
        return Err("431 Request Header Fields Too Large");
    }
    let text = std::str::from_utf8(head).map_err(|_| "400 Bad Request")?;
    let mut lines = text.split("\r\n");
    let line = lines.next().unwrap_or_default();
    let mut parts = line.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err("400 Bad Request");
    };
    if !version.starts_with("HTTP/1.") || !target.starts_with('/') {
        return Err("400 Bad Request");
    }
    let mut request = Request {
        method: method.to_owned(),
        target: target.to_owned(),
        ..Request::default()
    };
    for l in lines.take_while(|l| !l.is_empty()) {
        let Some((name, value)) = l.split_once(':') else {
            continue;
        };
        let slot = if name.eq_ignore_ascii_case("host") {
            &mut request.host
        } else if name.eq_ignore_ascii_case("origin") {
            &mut request.origin
        } else if name.eq_ignore_ascii_case("sec-fetch-site") {
            &mut request.fetch_site
        } else if name.eq_ignore_ascii_case("x-iohr-token") {
            &mut request.token
        } else if name.eq_ignore_ascii_case("cookie") {
            &mut request.cookie
        } else if name.eq_ignore_ascii_case("content-length") {
            &mut request.content_length
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            &mut request.transfer_encoding
        } else if name.eq_ignore_ascii_case("content-type") {
            &mut request.content_type
        } else if name.eq_ignore_ascii_case("accept") {
            &mut request.accept
        } else if name.eq_ignore_ascii_case("upgrade") {
            &mut request.upgrade
        } else if name.eq_ignore_ascii_case("sec-websocket-key") {
            &mut request.ws_key
        } else if name.eq_ignore_ascii_case("sec-websocket-version") {
            &mut request.ws_version
        } else {
            continue;
        };
        if slot.is_some() {
            return Err("400 Bad Request");
        }
        *slot = Some(value.trim().to_owned());
    }
    Ok(request)
}

/// A target's path, without its query or fragment, percent-decoded; `None` for a bad
/// escape.
fn decoded_path(target: &str) -> Option<String> {
    percent_decode(target.split(['?', '#']).next().unwrap_or_default())
}

/// Whether a target is under `/api`, judged on the decoded path so an escape cannot
/// slip past the caller check.
fn is_api(target: &str) -> bool {
    decoded_path(target).is_some_and(|p| p == "/api" || p.starts_with("/api/"))
}

/// `GET /api/extensions`: `{"extensions": [{name, version, title, entry, port?}]}`, the
/// installed extensions with a panel, by name, with `port` on the ones this serve
/// started. The store is read off the async threads; a lock that cannot be read is 500
/// with no detail on the wire.
async fn extensions(store: Arc<Store>, panels: &Panels) -> Reply {
    let listed = tokio::task::spawn_blocking(move || store.panels()).await;
    let Ok(Ok(listed)) = listed else {
        return Reply::error("500 Internal Server Error");
    };
    let ports = panels.ports();
    let mut entries = Vec::with_capacity(listed.len());
    for panel in listed {
        let port = ports.get(&panel.name).copied();
        let Ok(mut entry) = serde_json::to_value(panel) else {
            return Reply::error("500 Internal Server Error");
        };
        if let (Some(port), Some(fields)) = (port, entry.as_object_mut()) {
            fields.insert("port".to_owned(), port.into());
        }
        entries.push(entry);
    }
    Reply::json(&serde_json::json!({ "extensions": entries }))
}

/// An `/api/*` caller is this listener's own page: the browser says `Sec-Fetch-Site`
/// is `same-origin` (or `none`, typed by the person), and `Origin`, when sent, is
/// `http://` and a `Host` this listener answers to. Anything else, `null` included, is
/// another site's page.
fn api_caller_allowed(request: &Request, own: &str) -> bool {
    let site_ok = request.fetch_site.as_deref().is_some_and(|site| {
        site.eq_ignore_ascii_case("same-origin") || site.eq_ignore_ascii_case("none")
    });
    let origin_ok = request.origin.as_deref().is_none_or(|origin| {
        origin
            .strip_prefix("http://")
            .is_some_and(|h| host_allowed(Some(h), own))
    });
    site_ok && origin_ok
}

/// Whether `Host` names this listener: its own address, or `localhost` on its port,
/// compared whole (never by suffix); a missing `Host` is refused.
fn host_allowed(given: Option<&str>, own: &str) -> bool {
    let Some(given) = given else { return false };
    if given.eq_ignore_ascii_case(own) {
        return true;
    }
    let port = own.rsplit_once(':').map_or("", |(_, p)| p);
    given.eq_ignore_ascii_case(&format!("localhost:{port}"))
}

/// Why a path in a folder is not served.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outside {
    /// Absolute, `..`, NUL, a backslash or a drive: never a plain name in the folder.
    Malformed,
    /// A link, or something that resolves out of the folder or is not a file.
    Refused,
    /// Nothing there.
    Missing,
}

/// The file `relative` (`/`-separated, already decoded) names inside the canonical
/// folder `root`. A folder means its `index.html`. No part of the path may be a link,
/// and the canonical result must still be a file under `root`. Every UI file resolves
/// through this. (A panel's `entry` is a path on the panel's own server, RFC 0143, so
/// it never comes here.)
pub(crate) fn resolve_inside(root: &Path, relative: &str) -> Result<PathBuf, Outside> {
    if relative.starts_with('/') || relative.contains(['\0', '\\']) {
        return Err(Outside::Malformed);
    }
    let segments: Vec<&str> = relative
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    // A segment like `C:` would replace the root on Windows; plain names only.
    let plain = |s: &&str| {
        let mut parts = Path::new(s).components();
        matches!(
            (parts.next(), parts.next()),
            (Some(Component::Normal(_)), None)
        )
    };
    if !segments.iter().all(plain) {
        return Err(Outside::Malformed);
    }
    let mut candidate = root.to_path_buf();
    for segment in segments {
        candidate.push(segment);
        not_a_link(&candidate)?;
    }
    if candidate.is_dir() {
        candidate.push("index.html");
        not_a_link(&candidate)?;
    }
    let found = std::fs::canonicalize(&candidate).map_err(|_| Outside::Missing)?;
    if !found.starts_with(root) || !found.is_file() {
        return Err(Outside::Refused);
    }
    Ok(found)
}

fn not_a_link(path: &Path) -> Result<(), Outside> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Err(Outside::Refused),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(Outside::Missing),
        Err(_) => Err(Outside::Refused),
    }
}

/// The file a target names, inside `root` (canonical). An unknown page path (no
/// extension) gets the root's own `index.html` and nothing else; an unknown asset is
/// 404, so a broken build shows.
fn file_for(root: &Path, target: &str) -> Result<PathBuf, &'static str> {
    let decoded = decoded_path(target).ok_or("400 Bad Request")?;
    match resolve_inside(root, decoded.trim_start_matches('/')) {
        Ok(found) => Ok(found),
        Err(Outside::Malformed) => Err("400 Bad Request"),
        Err(Outside::Refused) => Err("404 Not Found"),
        Err(Outside::Missing) => {
            let page = decoded
                .rsplit('/')
                .next()
                .is_none_or(|last| !last.contains('.'));
            if page {
                resolve_inside(root, "index.html").map_err(|_| "404 Not Found")
            } else {
                Err("404 Not Found")
            }
        }
    }
}

/// `%XX` decoded; `None` for a bad escape or bytes that are not UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let hex = std::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

async fn serve_file(path: &Path) -> Reply {
    match tokio::fs::metadata(path).await {
        Ok(m) if m.len() <= MAX_FILE => {}
        Ok(_) => return Reply::error("403 Forbidden"),
        Err(_) => return Reply::error("404 Not Found"),
    }
    let Ok(body) = tokio::fs::read(path).await else {
        return Reply::error("404 Not Found");
    };
    let content_type = content_type(path);
    Reply {
        status: "200 OK",
        content_type,
        body,
        head_only: false,
        no_store: content_type.starts_with("text/html"),
        allow: false,
        frame_src: Vec::new(),
        headers: Vec::new(),
    }
}

fn content_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("txt") => "text/plain; charset=utf-8",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Mutex;

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpStream;

    use super::{
        Outside, Server, file_for, forget, host_allowed, listen_error, percent_decode,
        recorded_url, remember, resolve_inside, ui_package, ui_root,
    };
    use crate::ext::install::Store;
    use crate::ext::lock::{Entry, Lock};
    use crate::ext::oci::Digest;

    /// The session cookie line of each test server by address: `send` signs its request
    /// in with it, `send_bare` does not, so a test about the gate is the only one that
    /// says so.
    static COOKIES: Mutex<Option<HashMap<std::net::SocketAddr, String>>> = Mutex::new(None);

    pub(super) fn built_ui() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("index.html"), "<!doctype html><p>ui</p>").expect("index");
        std::fs::create_dir(dir.path().join("assets")).expect("assets");
        std::fs::write(dir.path().join("assets/app-1.js"), "console.log(1)").expect("js");
        dir
    }

    /// Starts a server on a free port, with no extension installed, and returns its
    /// address and a stop switch.
    pub(super) async fn start(
        root: &Path,
    ) -> (std::net::SocketAddr, tokio::sync::oneshot::Sender<()>) {
        start_with(root, Store::new(root.join("no-extensions"))).await
    }

    /// No panel started yet, with the default flags and an empty environment.
    fn panels() -> super::Panels {
        panels_with(&[], crate::Env::default())
    }

    /// Panels for a serve run with these global flags (before `ui serve`) and environment.
    pub(super) fn panels_with(flags: &[&str], env: crate::Env) -> super::Panels {
        use clap::Parser as _;
        let args = ["iohr"].into_iter().chain(flags.iter().copied());
        let cli = crate::cli::Cli::try_parse_from(args.chain(["ui", "serve"])).expect("cli");
        super::Panels::new(cli.global, env).expect("panels")
    }

    async fn start_with(
        root: &Path,
        store: Store,
    ) -> (std::net::SocketAddr, tokio::sync::oneshot::Sender<()>) {
        start_panels(root, store, panels()).await
    }

    /// A server with these stream and body limits, on a free port.
    pub(super) async fn start_limited(
        root: &Path,
        panels: super::Panels,
        limits: super::forward::Limits,
    ) -> (std::net::SocketAddr, tokio::sync::oneshot::Sender<()>) {
        let server = Server::bind_with(
            root,
            0,
            Store::new(root.join("no-extensions")),
            panels,
            limits,
        )
        .await
        .expect("bind");
        let addr = server.addr;
        COOKIES
            .lock()
            .expect("cookies")
            .get_or_insert_with(HashMap::new)
            .insert(addr, server.site.access.cookie_line());
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(server.serve(async {
            let _ = rx.await;
        }));
        (addr, tx)
    }

    pub(super) async fn start_panels(
        root: &Path,
        store: Store,
        panels: super::Panels,
    ) -> (std::net::SocketAddr, tokio::sync::oneshot::Sender<()>) {
        let server = Server::bind(root, 0, store, panels).await.expect("bind");
        let addr = server.addr;
        COOKIES
            .lock()
            .expect("cookies")
            .get_or_insert_with(HashMap::new)
            .insert(addr, server.site.access.cookie_line());
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(server.serve(async {
            let _ = rx.await;
        }));
        (addr, tx)
    }

    /// The launch secret of the test server at `addr`.
    pub(super) fn launch_of(server: std::net::SocketAddr) -> String {
        COOKIES
            .lock()
            .expect("cookies")
            .as_ref()
            .and_then(|m| m.get(&server))
            .and_then(|c| c.split_once('|').map(|(_, l)| l.to_owned()))
            .expect("a started server")
    }

    /// `raw` as a signed-in browser sends it: with the session cookie.
    pub(super) async fn send(addr: std::net::SocketAddr, raw: &str) -> String {
        let cookie = COOKIES
            .lock()
            .expect("cookies")
            .as_ref()
            .and_then(|m| m.get(&addr))
            .and_then(|c| c.split_once('|').map(|(c, _)| c.to_owned()))
            .expect("a started server");
        let raw = raw.replacen("\r\n", &format!("\r\nCookie: {cookie}\r\n"), 1);
        send_bare(addr, &raw).await
    }

    /// The session cookie of the test server at `addr`, as `name=value`.
    pub(super) fn cookie_of(addr: std::net::SocketAddr) -> String {
        COOKIES
            .lock()
            .expect("cookies")
            .as_ref()
            .and_then(|m| m.get(&addr))
            .and_then(|c| c.split_once('|').map(|(c, _)| c.to_owned()))
            .expect("a started server")
    }

    /// `raw` exactly as given.
    pub(super) async fn send_bare(addr: std::net::SocketAddr, raw: &str) -> String {
        let mut s = TcpStream::connect(addr).await.expect("connect");
        s.write_all(raw.as_bytes()).await.expect("write");
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.expect("read");
        String::from_utf8_lossy(&buf).into_owned()
    }

    pub(super) fn get(path: &str, host: &str) -> String {
        format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n")
    }

    #[tokio::test]
    async fn serves_index_assets_and_page_routes_with_strict_headers() {
        let dir = built_ui();
        let (addr, _stop) = start(dir.path()).await;
        let host = super::host_of(addr);

        let index = send(addr, &get("/", &host)).await;
        assert!(index.starts_with("HTTP/1.1 200 OK"), "{index}");
        assert!(index.contains("Content-Type: text/html"));
        assert!(index.contains("Cache-Control: no-store"));
        assert!(index.contains("frame-ancestors 'none'"));
        assert!(index.contains("X-Frame-Options: DENY"));
        assert!(index.contains("X-Content-Type-Options: nosniff"));
        assert!(index.contains("Referrer-Policy: no-referrer"));
        assert!(index.contains("Cross-Origin-Resource-Policy: same-origin"));
        assert!(index.ends_with("<p>ui</p>"));
        // The start token sits right after the doctype; nothing is framed yet.
        assert!(index.contains("<!doctype html><meta name=\"iohr-token\" content=\""));
        assert_eq!(token_of(&index).len(), 64);
        assert!(!index.contains("frame-src"));

        let js = send(addr, &get("/assets/app-1.js?v=1", &host)).await;
        assert!(js.starts_with("HTTP/1.1 200 OK"), "{js}");
        assert!(js.contains("Content-Type: text/javascript"));

        // An app route loads the app; a missing asset stays missing.
        let route = send(addr, &get("/projects/one", &host)).await;
        assert!(route.starts_with("HTTP/1.1 200 OK") && route.ends_with("<p>ui</p>"));
        let missing = send(addr, &get("/assets/gone.js", &host)).await;
        assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");

        let port = addr.port();
        let localhost = send(addr, &get("/", &format!("localhost:{port}"))).await;
        assert!(localhost.starts_with("HTTP/1.1 200 OK"));

        let head = send(addr, &format!("HEAD / HTTP/1.1\r\nHost: {host}\r\n\r\n")).await;
        assert!(
            head.starts_with("HTTP/1.1 200 OK") && head.ends_with("\r\n\r\n"),
            "{head}"
        );
    }

    #[tokio::test]
    async fn refuses_other_hosts_methods_and_paths_out_of_the_folder() {
        let dir = built_ui();
        let (addr, _stop) = start(dir.path()).await;
        let host = super::host_of(addr);

        // DNS rebinding: a page on another name reaching this port.
        let rebound = send(addr, &get("/", &format!("evil.example:{}", addr.port()))).await;
        assert!(rebound.starts_with("HTTP/1.1 403"), "{rebound}");
        let no_host = send(addr, "GET / HTTP/1.1\r\n\r\n").await;
        assert!(no_host.starts_with("HTTP/1.1 403"), "{no_host}");
        let other_port = send(addr, &get("/", "127.0.0.1:1")).await;
        assert!(other_port.starts_with("HTTP/1.1 403"));

        let post = send(addr, &format!("POST / HTTP/1.1\r\nHost: {host}\r\n\r\n")).await;
        assert!(post.starts_with("HTTP/1.1 405") && post.contains("Allow: GET, HEAD"));

        for path in [
            "/../secret",
            "/assets/%2e%2e/%2e%2e/x",
            "/a%5cb",
            "/a%00b",
            "/%zz",
        ] {
            let r = send(addr, &get(path, &host)).await;
            assert!(r.starts_with("HTTP/1.1 400"), "{path}: {r}");
        }
        let absolute = send(addr, &get("http://127.0.0.1/", &host)).await;
        assert!(absolute.starts_with("HTTP/1.1 400"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_link_out_of_the_folder_is_not_followed() {
        let dir = built_ui();
        let outside = tempfile::tempdir().expect("outside");
        std::fs::write(outside.path().join("secret.txt"), "secret").expect("secret");
        std::os::unix::fs::symlink(outside.path().join("secret.txt"), dir.path().join("s.txt"))
            .expect("link");
        let (addr, _stop) = start(dir.path()).await;
        let r = send(addr, &get("/s.txt", &super::host_of(addr))).await;
        assert!(r.starts_with("HTTP/1.1 404") && !r.contains("secret\n") && !r.ends_with("secret"));
    }

    #[test]
    fn the_root_is_the_flag_or_the_nearest_built_ui() {
        let repo = tempfile::tempdir().expect("repo");
        let dist = repo.path().join("ui/dist");
        std::fs::create_dir_all(&dist).expect("dist");
        std::fs::write(dist.join("index.html"), "x").expect("index");
        let deep = repo.path().join("cli/crates");
        std::fs::create_dir_all(&deep).expect("deep");
        let found = ui_root(None, &deep).expect("found");
        assert_eq!(found, std::fs::canonicalize(&dist).expect("canon"));
        assert!(
            ui_root(Some(&deep), &deep).is_err(),
            "a folder without index.html"
        );
        assert_eq!(ui_root(Some(&dist), Path::new("/")).expect("flag"), found);
    }

    #[test]
    fn host_decoding_and_paths() {
        assert!(host_allowed(Some("127.0.0.1:5"), "127.0.0.1:5"));
        assert!(host_allowed(Some("LOCALHOST:5"), "127.0.0.1:5"));
        assert!(host_allowed(Some("[::1]:5"), "[::1]:5"));
        assert!(!host_allowed(Some("localhost"), "127.0.0.1:5"));
        assert!(!host_allowed(None, "127.0.0.1:5"));
        assert_eq!(percent_decode("/a%20b").as_deref(), Some("/a b"));
        assert_eq!(percent_decode("/a%2"), None);
        assert_eq!(percent_decode("/%ff"), None);
        let dir = built_ui();
        let root = std::fs::canonicalize(dir.path()).expect("canon");
        assert_eq!(
            file_for(&root, "/").expect("index"),
            root.join("index.html")
        );
        assert_eq!(
            file_for(&root, "/assets").expect("dir index falls back"),
            root.join("index.html")
        );
    }

    fn api(path: &str, host: &str, extra: &str) -> String {
        format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n{extra}\r\n")
    }

    #[tokio::test]
    async fn api_answers_only_this_listeners_own_pages() {
        let dir = built_ui();
        let (addr, _stop) = start(dir.path()).await;
        let host = super::host_of(addr);
        let own = format!("Origin: http://{host}\r\n");

        // The UI's own fetch: allowed through. Nothing installed is an empty list.
        let same = send(
            addr,
            &api(
                "/api/extensions",
                &host,
                &format!("Sec-Fetch-Site: same-origin\r\n{own}"),
            ),
        )
        .await;
        assert!(same.starts_with("HTTP/1.1 200"), "{same}");
        assert!(same.ends_with("\r\n\r\n{\"extensions\":[]}"), "{same}");
        assert!(same.contains("Cache-Control: no-store") && !same.contains("<p>ui</p>"));
        assert!(same.contains("Cross-Origin-Resource-Policy: same-origin"));
        // Any other `/api` path is 404, never the app page.
        for other in ["/api", "/api/", "/api/extensions/x", "/api/other"] {
            let r = send(addr, &api(other, &host, "Sec-Fetch-Site: none\r\n")).await;
            assert!(
                r.starts_with("HTTP/1.1 404") && !r.contains("<p>ui</p>"),
                "{other}: {r}"
            );
        }

        for extra in [
            String::new(),
            "Sec-Fetch-Site: cross-site\r\n".to_owned(),
            "Sec-Fetch-Site: same-site\r\n".to_owned(),
            "Sec-Fetch-Site: same-origin\r\nOrigin: http://evil.example\r\n".to_owned(),
            "Sec-Fetch-Site: same-origin\r\nOrigin: null\r\n".to_owned(),
            format!("Sec-Fetch-Site: same-origin\r\nOrigin: https://{host}\r\n"),
            format!("Sec-Fetch-Site: same-origin\r\nOrigin: http://{host}.evil.example\r\n"),
        ] {
            let r = send(addr, &api("/api/extensions", &host, &extra)).await;
            assert!(r.starts_with("HTTP/1.1 403"), "{extra}: {r}");
        }
        // An escaped `/api` is still `/api`.
        let escaped = send(addr, &api("/%61pi/extensions", &host, "")).await;
        assert!(escaped.starts_with("HTTP/1.1 403"), "{escaped}");
        let twice = send(
            addr,
            &api(
                "/api/x",
                &host,
                "Sec-Fetch-Site: none\r\nSec-Fetch-Site: none\r\n",
            ),
        )
        .await;
        assert!(twice.starts_with("HTTP/1.1 400"), "{twice}");
    }

    /// Lays `name` out in the store as `iohr ext install` does: a lock entry and the
    /// version's `record.json` (no program: listing never runs one). `record: false`
    /// leaves the record out, as a broken install would.
    fn installed(store: &Path, name: &str, panel: Option<serde_json::Value>, record: bool) {
        let digest = Digest::of(name.as_bytes());
        let lock_path = store.join("iohr-ext.lock");
        let mut lock = Lock::load(&lock_path).expect("lock");
        let entry = Entry::new(name, "0.1.0", &digest, "test signer".into(), Vec::new());
        lock.entries.insert(name.to_owned(), entry);
        std::fs::create_dir_all(store).expect("store");
        lock.save(&lock_path).expect("save");
        if !record {
            return;
        }
        let mut manifest = serde_json::json!({
            "name": name, "version": "0.1.0", "entrypoint": format!("bin/{name}"),
        });
        if let Some(p) = panel {
            manifest["panel"] = p;
        }
        let dir = store.join(name).join(format!("0.1.0-{}", digest.short()));
        std::fs::create_dir_all(&dir).expect("version dir");
        let sha = Digest::of(b"program").to_string();
        let record = serde_json::json!({
            "manifest": manifest, "index": digest.to_string(),
            "platform_manifest": digest.to_string(), "program": name,
            "program_sha256": sha, "signer": "test signer",
            "registry": "registry.example/ext", "installed_at": "2026-11-09T00:00:00Z",
        });
        std::fs::write(dir.join("record.json"), record.to_string()).expect("record");
    }

    #[tokio::test]
    async fn api_extensions_lists_only_installed_panels_that_pass_the_rule() {
        let dir = built_ui();
        let data = tempfile::tempdir().expect("data");
        let store = data.path().join("extensions");
        installed(
            &store,
            "todo",
            Some(serde_json::json!({"title": "Todo", "entry": "/"})),
            true,
        );
        installed(
            &store,
            "board",
            Some(serde_json::json!({"title": "Board", "entry": "/app/"})),
            true,
        );
        installed(&store, "agent", None, true);
        installed(
            &store,
            "evil",
            Some(serde_json::json!({"title": "Evil", "entry": "//evil.example/"})),
            true,
        );
        installed(
            &store,
            "broken",
            Some(serde_json::json!({"title": "B", "entry": "/"})),
            false,
        );
        let (addr, _stop) = start_with(dir.path(), Store::in_data_dir(data.path())).await;
        let host = super::host_of(addr);

        let r = send(
            addr,
            &api(
                "/api/extensions?fresh=1",
                &host,
                "Sec-Fetch-Site: same-origin\r\n",
            ),
        )
        .await;
        assert!(r.starts_with("HTTP/1.1 200"), "{r}");
        assert!(r.contains("Content-Type: application/json\r\n"), "{r}");
        assert!(r.contains("Cache-Control: no-store"), "{r}");
        let body = r.split_once("\r\n\r\n").map(|(_, b)| b).expect("body");
        let got: serde_json::Value = serde_json::from_str(body).expect("json");
        assert_eq!(
            got,
            serde_json::json!({"extensions": [
                {"name": "board", "version": "0.1.0", "title": "Board", "entry": "/app/"},
                {"name": "todo", "version": "0.1.0", "title": "Todo", "entry": "/"},
            ]})
        );

        let head = send(
            addr,
            &format!(
                "HEAD /api/extensions HTTP/1.1\r\nHost: {host}\r\nSec-Fetch-Site: none\r\n\r\n"
            ),
        )
        .await;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(
            head.contains(&format!("Content-Length: {}\r\n", body.len())),
            "{head}"
        );
        assert!(head.ends_with("\r\n\r\n"), "{head}");

        // The caller check still runs first: another site sees none of it.
        let cross = send(
            addr,
            &api("/api/extensions", &host, "Sec-Fetch-Site: cross-site\r\n"),
        )
        .await;
        assert!(
            cross.starts_with("HTTP/1.1 403") && !cross.contains("todo"),
            "{cross}"
        );

        // A lock that cannot be read is 500 with nothing of it on the wire.
        std::fs::write(store.join("iohr-ext.lock"), "not a lock").expect("garble");
        let bad = send(
            addr,
            &api("/api/extensions", &host, "Sec-Fetch-Site: none\r\n"),
        )
        .await;
        assert!(
            bad.starts_with("HTTP/1.1 500") && !bad.contains("iohr-ext.lock"),
            "{bad}"
        );
    }

    #[tokio::test]
    async fn a_taken_port_is_an_error_not_another_address() {
        let dir = built_ui();
        let held = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("hold");
        let port = held.local_addr().expect("addr").port();
        let store = Store::new(dir.path().join("none"));
        let Err(e) = Server::bind(dir.path(), port, store, panels()).await else {
            panic!("bound a taken port");
        };
        assert_eq!(e.kind(), std::io::ErrorKind::AddrInUse);
        let message = listen_error(port, &e).to_string();
        assert!(
            message.contains(&format!("port {port} is already in use")),
            "{message}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn links_are_refused_even_inside_and_a_linked_root_still_serves() {
        let dir = built_ui();
        let root = std::fs::canonicalize(dir.path()).expect("canon");
        std::os::unix::fs::symlink(root.join("assets/app-1.js"), root.join("inner.js"))
            .expect("inner link");
        std::os::unix::fs::symlink(root.join("assets"), root.join("linked")).expect("dir link");
        assert_eq!(resolve_inside(&root, "inner.js"), Err(Outside::Refused));
        assert_eq!(
            resolve_inside(&root, "linked/app-1.js"),
            Err(Outside::Refused)
        );

        // The root itself may be reached through a link: it is canonical from the start.
        let parent = tempfile::tempdir().expect("parent");
        let via = parent.path().join("ui");
        std::os::unix::fs::symlink(&root, &via).expect("root link");
        let (addr, _stop) = start(&via).await;
        let r = send(addr, &get("/assets/app-1.js", &super::host_of(addr))).await;
        assert!(r.starts_with("HTTP/1.1 200 OK"), "{r}");
    }

    #[test]
    fn a_path_resolves_only_to_a_plain_file_in_the_folder() {
        let dir = built_ui();
        let root = std::fs::canonicalize(dir.path()).expect("canon");
        assert_eq!(
            resolve_inside(&root, "assets/app-1.js"),
            Ok(root.join("assets/app-1.js"))
        );
        for bad in ["/etc/passwd", "../x", "assets/../../x", "a\\b", "a\0b"] {
            assert_eq!(resolve_inside(&root, bad), Err(Outside::Malformed), "{bad}");
        }
        assert_eq!(resolve_inside(&root, "nope.js"), Err(Outside::Missing));
    }

    #[test]
    fn the_package_is_the_flag_or_the_nearest_ui_with_a_package_json() {
        let repo = tempfile::tempdir().expect("repo");
        let ui = repo.path().join("ui");
        std::fs::create_dir_all(&ui).expect("ui");
        let deep = repo.path().join("cli/crates");
        std::fs::create_dir_all(&deep).expect("deep");
        assert!(ui_package(None, &deep).is_err(), "no package.json yet");
        std::fs::write(ui.join("package.json"), "{}").expect("package");
        let found = ui_package(None, &deep).expect("found");
        assert_eq!(found, std::fs::canonicalize(&ui).expect("canon"));
        assert_eq!(ui_package(Some(&ui), Path::new("/")).expect("flag"), found);
        assert!(
            ui_package(Some(&deep), &deep).is_err(),
            "a folder without package.json"
        );
    }

    #[test]
    fn only_an_exact_loopback_url_with_a_port_is_taken_from_the_record() {
        let ok = |url: &str| recorded_url(&format!(r#"{{"url":"{url}"}}"#)).map(|(u, _)| u);
        assert_eq!(
            ok("http://127.0.0.1:4321/").as_deref(),
            Some("http://127.0.0.1:4321/")
        );
        assert_eq!(
            ok("http://[::1]:4321/").as_deref(),
            Some("http://[::1]:4321/")
        );
        for bad in [
            "http://127.0.0.1:0/",
            "http://127.0.0.1:4321",
            "http://127.0.0.1:4321/x",
            "https://127.0.0.1:4321/",
            "http://localhost:4321/",
            "http://10.1.2.3:4321/",
            "http://0.0.0.0:4321/",
            "http://127.0.0.1:04321/",
            "http://user@127.0.0.1:4321/",
            "javascript:alert(1)",
        ] {
            assert_eq!(ok(bad), None, "{bad}");
        }
        assert!(recorded_url("not json").is_none());
        assert!(recorded_url(r#"{"url":7}"#).is_none());
    }

    #[test]
    fn the_record_is_private_and_removed_only_by_its_own_server() {
        let data = tempfile::tempdir().expect("data");
        let folder = data.path().join("ui");
        let launch = "ab".repeat(32);
        remember(&folder, "http://127.0.0.1:4321/", data.path(), &launch).expect("remember");
        let path = folder.join("serving.json");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let text = std::fs::read_to_string(&path).expect("read");
        let (url, _) = recorded_url(&text).expect("valid");
        assert_eq!(url, "http://127.0.0.1:4321/");
        assert_eq!(super::recorded_launch(&text), Some(launch));
        assert_eq!(super::recorded_launch(r#"{"launch":"short"}"#), None);
        assert_eq!(
            super::recorded_launch(&format!(r#"{{"launch":"{}"}}"#, "z".repeat(64))),
            None
        );
        forget(&folder, "http://127.0.0.1:9999/");
        assert!(path.exists(), "another server's url leaves the record");
        forget(&folder, "http://127.0.0.1:4321/");
        assert!(!path.exists(), "its own url removes it");
    }

    /// The start token in a page answer.
    pub(super) fn token_of(page: &str) -> String {
        let at = page
            .find("name=\"iohr-token\" content=\"")
            .expect("token meta")
            + "name=\"iohr-token\" content=\"".len();
        page[at..]
            .chars()
            .take_while(char::is_ascii_hexdigit)
            .collect()
    }

    /// A start or stop as the UI's page sends it, with `extra` headers after.
    pub(super) fn post(path: &str, host: &str, token: &str, extra: &str) -> String {
        format!(
            "POST {path} HTTP/1.1\r\nHost: {host}\r\nSec-Fetch-Site: same-origin\r\n\
             Origin: http://{host}\r\nx-iohr-token: {token}\r\nContent-Length: 0\r\n{extra}\r\n"
        )
    }

    /// Gives installed `name` a real program: it writes its pid to `pid`, prints `line`
    /// and sleeps, and the record's digest is the script's, so the trust check passes.
    fn program(store: &Path, name: &str, line: &str, pid: &Path) {
        use std::os::unix::fs::PermissionsExt as _;
        let digest = Digest::of(name.as_bytes());
        let dir = store.join(name).join(format!("0.1.0-{}", digest.short()));
        let script = format!(
            "#!/bin/sh\necho $$ > '{}'\necho '{line}'\nexec sleep 60\n",
            pid.display()
        );
        let path = dir.join(name);
        std::fs::write(&path, &script).expect("program");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("mode");
        let record_path = dir.join("record.json");
        let mut record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&record_path).expect("record")).expect("json");
        record["program_sha256"] = Digest::of(script.as_bytes()).to_string().into();
        std::fs::write(&record_path, record.to_string()).expect("record");
    }

    /// Whether the process `pid` names (written by [`program`]) is still there.
    fn alive(pid: &Path) -> bool {
        let pid = std::fs::read_to_string(pid).expect("pid");
        std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(std::process::Stdio::null())
            .status()
            .expect("kill")
            .success()
    }

    fn panel(title: &str) -> serde_json::Value {
        serde_json::json!({"title": title, "entry": "/"})
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_started_panel_runs_once_is_framed_and_stops() {
        let dir = built_ui();
        let data = tempfile::tempdir().expect("data");
        let store = data.path().join("extensions");
        installed(&store, "todo", Some(panel("Todo")), true);
        let held = std::net::TcpListener::bind("127.0.0.1:0").expect("panel port");
        let port = held.local_addr().expect("addr").port();
        let pid = data.path().join("todo.pid");
        program(&store, "todo", &format!("iohr-panel {port}"), &pid);
        let (addr, stop) = start_with(dir.path(), Store::new(store)).await;
        let host = super::host_of(addr);
        let token = token_of(&send(addr, &get("/", &host)).await);

        let started = send(addr, &post("/api/extensions/todo/start", &host, &token, "")).await;
        assert!(started.starts_with("HTTP/1.1 200"), "{started}");
        assert!(
            started.ends_with(&format!("{{\"port\":{port}}}")),
            "{started}"
        );
        assert!(alive(&pid));
        // One instance: a second start answers the same port.
        let again = send(addr, &post("/api/extensions/todo/start", &host, &token, "")).await;
        assert!(again.ends_with(&format!("{{\"port\":{port}}}")), "{again}");

        let own = format!("Sec-Fetch-Site: same-origin\r\nOrigin: http://{host}\r\n");
        let listed = send(addr, &api("/api/extensions", &host, &own)).await;
        assert!(listed.contains(&format!("\"port\":{port}")), "{listed}");
        // Pages may frame exactly that port; the UI itself is never framed.
        let page = send(addr, &get("/ext/todo", &host)).await;
        assert!(
            page.contains(&format!(
                "frame-ancestors 'none'; frame-src http://127.0.0.1:{port}\r\n"
            )),
            "{page}"
        );

        let stopped = send(addr, &post("/api/extensions/todo/stop", &host, &token, "")).await;
        assert!(stopped.ends_with("{\"stopped\":true}"), "{stopped}");
        assert!(!alive(&pid), "the panel still runs after stop");
        let page = send(addr, &get("/", &host)).await;
        assert!(!page.contains("frame-src"), "{page}");

        // Stopping the serve stops what it started.
        let restarted = send(addr, &post("/api/extensions/todo/start", &host, &token, "")).await;
        assert!(restarted.starts_with("HTTP/1.1 200"), "{restarted}");
        assert!(alive(&pid));
        let _ = stop.send(());
        let mut gone = false;
        for _ in 0..100 {
            if !alive(&pid) {
                gone = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(gone, "the panel outlived the serve");
        drop(held);
    }

    #[tokio::test]
    async fn a_start_needs_the_pages_origin_token_and_an_installed_name() {
        let dir = built_ui();
        let data = tempfile::tempdir().expect("data");
        let store = data.path().join("extensions");
        installed(&store, "todo", Some(panel("Todo")), true);
        let (addr, _stop) = start_with(dir.path(), Store::new(store)).await;
        let host = super::host_of(addr);
        let token = token_of(&send(addr, &get("/", &host)).await);
        let wrong = "0".repeat(64);
        let path = "/api/extensions/todo/start";
        let bare = |headers: &str| format!("POST {path} HTTP/1.1\r\nHost: {host}\r\n{headers}\r\n");

        for (why, raw, status) in [
            ("no headers", bare(""), "403"),
            (
                "no token",
                bare(&format!(
                    "Sec-Fetch-Site: same-origin\r\nOrigin: http://{host}\r\n"
                )),
                "403",
            ),
            ("wrong token", post(path, &host, &wrong, ""), "403"),
            ("short token", post(path, &host, &token[..63], ""), "403"),
            (
                "another origin",
                bare(&format!(
                    "Sec-Fetch-Site: same-origin\r\nOrigin: http://localhost:{}\r\nx-iohr-token: {token}\r\n",
                    addr.port()
                )),
                "403",
            ),
            (
                "cross-site",
                bare(&format!(
                    "Sec-Fetch-Site: cross-site\r\nOrigin: http://{host}\r\nx-iohr-token: {token}\r\n"
                )),
                "403",
            ),
            (
                "two tokens",
                post(path, &host, &token, &format!("x-iohr-token: {token}\r\n")),
                "400",
            ),
            (
                "a body",
                bare(&format!(
                    "Sec-Fetch-Site: same-origin\r\nOrigin: http://{host}\r\nx-iohr-token: {token}\r\nContent-Length: 2\r\n"
                )),
                "400",
            ),
            (
                "chunked",
                post(path, &host, &token, "Transfer-Encoding: chunked\r\n"),
                "400",
            ),
            (
                "a query",
                post(&format!("{path}?x=1"), &host, &token, ""),
                "404",
            ),
            (
                "escaped",
                post("/api/extensions/t%6fdo/start", &host, &token, ""),
                "404",
            ),
            (
                "a path",
                post("/api/extensions/../start", &host, &token, ""),
                "404",
            ),
            (
                "unknown",
                post("/api/extensions/nope/start", &host, &token, ""),
                "404",
            ),
            (
                "other verb",
                post("/api/extensions/todo/run", &host, &token, ""),
                "404",
            ),
            (
                "the list",
                post("/api/extensions", &host, &token, ""),
                "404",
            ),
        ] {
            let r = send(addr, &raw).await;
            assert!(r.starts_with(&format!("HTTP/1.1 {status}")), "{why}: {r}");
        }
        // Installed, but its program is not there: the trust check refuses, no detail.
        let r = send(addr, &post(path, &host, &token, "")).await;
        assert!(r.starts_with("HTTP/1.1 502") && !r.contains("todo"), "{r}");
        // Stopping what never ran is not an error.
        let r = send(addr, &post("/api/extensions/todo/stop", &host, &token, "")).await;
        assert!(r.ends_with("{\"stopped\":false}"), "{r}");
        // A start is not a GET.
        let r = send(addr, &get(path, &host)).await;
        assert!(
            r.starts_with("HTTP/1.1 403") || r.starts_with("HTTP/1.1 404"),
            "{r}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_panel_naming_a_bad_or_taken_port_is_stopped() {
        let dir = built_ui();
        let data = tempfile::tempdir().expect("data");
        let store = data.path().join("extensions");
        for name in ["first", "second"] {
            installed(&store, name, Some(panel(name)), true);
        }
        let held = std::net::TcpListener::bind("127.0.0.1:0").expect("panel port");
        let port = held.local_addr().expect("addr").port();
        let free = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").expect("free port");
            l.local_addr().expect("addr").port()
        };
        let first_pid = data.path().join("first.pid");
        program(&store, "first", &format!("iohr-panel {port}"), &first_pid);
        let (addr, _stop) = start_with(dir.path(), Store::new(store.clone())).await;
        let host = super::host_of(addr);
        let token = token_of(&send(addr, &get("/", &host)).await);
        let first = send(
            addr,
            &post("/api/extensions/first/start", &host, &token, ""),
        )
        .await;
        assert!(first.starts_with("HTTP/1.1 200"), "{first}");

        let pid = data.path().join("second.pid");
        for line in [
            format!("iohr-panel {port}"),
            format!("iohr-panel {}", addr.port()),
            format!("iohr-panel {free}"),
            "iohr-panel 80".to_owned(),
            "iohr-panel 70000".to_owned(),
            format!("iohr-panel {port} extra"),
            "listening".to_owned(),
        ] {
            program(&store, "second", &line, &pid);
            let r = send(
                addr,
                &post("/api/extensions/second/start", &host, &token, ""),
            )
            .await;
            assert!(r.starts_with("HTTP/1.1 502"), "{line}: {r}");
            assert!(!alive(&pid), "{line}: the refused panel still runs");
        }
        // The first panel was never touched.
        assert!(alive(&first_pid));
        drop(held);
    }
}
