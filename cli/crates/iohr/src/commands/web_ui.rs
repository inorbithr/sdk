//! `iohr ui`: serve the InOrbit web UI (`sdk/ui`) on a loopback address (RFC 0217).
//!
//! The listener is the sign-in redirect's (`iohr_auth::loopback`): 127.0.0.1 first, else
//! `[::1]`, a read window and a head cap per connection. It adds what a file server
//! needs and no HTTP stack: GET and HEAD only, a `Host` that names this listener (so a
//! rebound DNS name cannot reach it), paths that cannot leave the UI folder or follow a
//! link, `/api/*` only from this listener's own pages (`Sec-Fetch-Site` and `Origin`), a
//! capped number of connections, and the same strict headers on every answer.
//!
//! `build`, `dev` and `check` run the UI package's own scripts through pnpm (the runner
//! `iohr sdk add` uses: no shell, the exit code passed on), so a developer, mise and CI
//! call one thing. A running `serve` records its address in the data folder
//! (`ui/serving.json`, 0600) for `iohr ui open`, and removes it when it stops.
//!
//! The one API route, `GET /api/extensions`, lists the installed extensions that serve
//! a panel (`ext::install::Store::panels`, the store `iohr ext install` writes). It
//! reads the lock and records only: nothing is started from here.

use std::io::{self, Read as _};
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use iohr_auth::loopback::{MAX_HEAD, READ_WINDOW, bind_loopback, read_head};
use tokio::io::AsyncWriteExt as _;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::time::timeout;

use crate::cli::{Global, UiArgs, UiCommand, UiPackage, UiServe};
use crate::commands::sdk_add::{resolve, run_program};
use crate::commands::session::data_dir;
use crate::error::Error;
use crate::ext::install::Store;
use crate::output::Out;
use crate::ui::Ui;

/// Connections answered at once; one more is closed unanswered.
const MAX_CONNECTIONS: usize = 16;
/// How long one answer may take to write.
const WRITE_WINDOW: Duration = Duration::from_secs(30);
/// The largest file served; a built UI's biggest chunk is far smaller.
const MAX_FILE: u64 = 32 * 1024 * 1024;
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
pub(crate) async fn run(g: &Global, args: UiArgs, out: Out) -> Result<(), Error> {
    match args.command {
        None => {
            let serve = UiServe {
                open: true,
                ..UiServe::default()
            };
            serve_until_stopped(g, &serve, out).await
        }
        Some(UiCommand::Serve(serve)) => serve_until_stopped(g, &serve, out).await,
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
    if out.json {
        Out::print_json(&serde_json::json!({ "url": url }));
    }
    if !crate::browser::open(&url) {
        if !out.json {
            Out::raw(format!("{url}\n").as_bytes());
        }
        Out::note(&format!("Open {url} in a browser."));
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

/// Records `url` (served from `root`) for `iohr ui open`.
fn remember(folder: &Path, url: &str, root: &Path) -> io::Result<()> {
    let body = serde_json::json!({
        "url": url,
        "dir": root.display().to_string(),
        "pid": std::process::id(),
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

async fn serve_until_stopped(g: &Global, args: &UiServe, out: Out) -> Result<(), Error> {
    let root = ui_root(args.dir.as_deref(), &working_folder()?)?;
    let port = args.port.unwrap_or(0);
    let server = Server::bind(&root, port, super::ext::store(g)?)
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
        Ok(f) => remember(f, &url, &root).map_err(|e| e.kind().to_string()),
        Err(e) => Err(e.to_string()),
    };
    if let Err(why) = &recorded {
        Out::note(&format!(
            "`iohr ui open` will not find this server ({why})."
        ));
    }
    if args.open && !crate::browser::open(&url) {
        Out::note(&format!("Open {url} in a browser."));
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

/// The UI listener: a canonical root, the extension store it lists, a loopback socket.
pub(crate) struct Server {
    listener: TcpListener,
    root: Arc<PathBuf>,
    store: Arc<Store>,
    addr: SocketAddr,
}

impl Server {
    /// Binds `port` (0: the system picks) on loopback, serving the canonical `root` and
    /// listing the panels in `store`.
    pub(crate) async fn bind(root: &Path, port: u16, store: Store) -> io::Result<Self> {
        let root = Arc::new(std::fs::canonicalize(root)?);
        let listener = bind_loopback(port).await?;
        let addr = listener.local_addr()?;
        Ok(Self {
            listener,
            root,
            store: Arc::new(store),
            addr,
        })
    }

    /// The address a browser opens.
    pub(crate) fn url(&self) -> String {
        format!("http://{}/", host_of(self.addr))
    }

    /// Answers connections until `stop` finishes.
    pub(crate) async fn serve(self, stop: impl Future<Output = ()>) {
        let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        let host = Arc::new(host_of(self.addr));
        tokio::pin!(stop);
        loop {
            tokio::select! {
                () = &mut stop => return,
                accepted = self.listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue };
                    // Over the cap: the connection closes unanswered.
                    let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else { continue };
                    let root = Arc::clone(&self.root);
                    let store = Arc::clone(&self.store);
                    let host = Arc::clone(&host);
                    tokio::spawn(async move {
                        answer(stream, &root, store, &host).await;
                        drop(permit);
                    });
                }
            }
        }
    }
}

/// `127.0.0.1:<port>` or `[::1]:<port>`, as a `Host` header names it.
fn host_of(addr: SocketAddr) -> String {
    match addr {
        SocketAddr::V4(a) => format!("{}:{}", a.ip(), a.port()),
        SocketAddr::V6(a) => format!("[{}]:{}", a.ip(), a.port()),
    }
}

async fn answer(mut stream: TcpStream, root: &Path, store: Arc<Store>, host: &str) {
    let Ok(Ok(head)) = timeout(READ_WINDOW, read_head(&mut stream)).await else {
        return;
    };
    let reply = reply_to(&head, root, store, host).await;
    let _ = timeout(WRITE_WINDOW, reply.write(&mut stream)).await;
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
        let head = format!(
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{allow}\
             Content-Security-Policy: {CSP}\r\n\
             Referrer-Policy: no-referrer\r\nCache-Control: {cache}\r\n\
             X-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\n\
             Cross-Origin-Opener-Policy: same-origin\r\n\
             Cross-Origin-Resource-Policy: same-origin\r\n\
             Connection: close\r\n\r\n",
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

/// The answer to one request head.
async fn reply_to(head: &[u8], root: &Path, store: Arc<Store>, host: &str) -> Reply {
    let request = match parse(head) {
        Ok(r) => r,
        Err(status) => return Reply::error(status),
    };
    if !host_allowed(request.host.as_deref(), host) {
        return Reply::error("403 Forbidden");
    }
    let head_only = match request.method.as_str() {
        "GET" => false,
        "HEAD" => true,
        _ => {
            return Reply {
                allow: true,
                ..Reply::error("405 Method Not Allowed")
            };
        }
    };
    if is_api(&request.target) {
        if !api_caller_allowed(&request, host) {
            return Reply::error("403 Forbidden");
        }
        // Any other `/api` path is 404, never the app page.
        let mut reply = match decoded_path(&request.target).as_deref() {
            Some("/api/extensions") => extensions(store).await,
            _ => Reply::error("404 Not Found"),
        };
        reply.head_only = head_only;
        return reply;
    }
    let mut reply = match file_for(root, &request.target) {
        Ok(path) => serve_file(&path).await,
        Err(status) => Reply::error(status),
    };
    reply.head_only = head_only;
    reply
}

#[derive(Debug, Default)]
struct Request {
    method: String,
    target: String,
    host: Option<String>,
    origin: Option<String>,
    fetch_site: Option<String>,
}

/// The request line and the `Host`, `Origin` and `Sec-Fetch-Site` headers of a head; an
/// error is the status to answer. Any of the three sent twice is refused.
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

/// `GET /api/extensions`: `{"extensions": [{name, version, title, entry}]}`, the
/// installed extensions with a panel, by name. The store is read off the async
/// threads; a lock that cannot be read is 500 with no detail on the wire.
async fn extensions(store: Arc<Store>) -> Reply {
    let listed = tokio::task::spawn_blocking(move || store.panels()).await;
    let Ok(Ok(panels)) = listed else {
        return Reply::error("500 Internal Server Error");
    };
    let Ok(body) = serde_json::to_vec(&serde_json::json!({ "extensions": panels })) else {
        return Reply::error("500 Internal Server Error");
    };
    Reply {
        status: "200 OK",
        content_type: "application/json",
        body,
        head_only: false,
        no_store: true,
        allow: false,
    }
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
    use std::path::Path;

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpStream;

    use super::{
        Outside, Server, file_for, forget, host_allowed, listen_error, percent_decode,
        recorded_url, remember, resolve_inside, ui_package, ui_root,
    };
    use crate::ext::install::Store;
    use crate::ext::lock::{Entry, Lock};
    use crate::ext::oci::Digest;

    fn built_ui() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("index.html"), "<!doctype html><p>ui</p>").expect("index");
        std::fs::create_dir(dir.path().join("assets")).expect("assets");
        std::fs::write(dir.path().join("assets/app-1.js"), "console.log(1)").expect("js");
        dir
    }

    /// Starts a server on a free port, with no extension installed, and returns its
    /// address and a stop switch.
    async fn start(root: &Path) -> (std::net::SocketAddr, tokio::sync::oneshot::Sender<()>) {
        start_with(root, Store::new(root.join("no-extensions"))).await
    }

    async fn start_with(
        root: &Path,
        store: Store,
    ) -> (std::net::SocketAddr, tokio::sync::oneshot::Sender<()>) {
        let server = Server::bind(root, 0, store).await.expect("bind");
        let addr = server.addr;
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(server.serve(async {
            let _ = rx.await;
        }));
        (addr, tx)
    }

    async fn send(addr: std::net::SocketAddr, raw: &str) -> String {
        let mut s = TcpStream::connect(addr).await.expect("connect");
        s.write_all(raw.as_bytes()).await.expect("write");
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.expect("read");
        String::from_utf8_lossy(&buf).into_owned()
    }

    fn get(path: &str, host: &str) -> String {
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
        let Err(e) = Server::bind(dir.path(), port, Store::new(dir.path().join("none"))).await
        else {
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
        remember(&folder, "http://127.0.0.1:4321/", data.path()).expect("remember");
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
        forget(&folder, "http://127.0.0.1:9999/");
        assert!(path.exists(), "another server's url leaves the record");
        forget(&folder, "http://127.0.0.1:4321/");
        assert!(!path.exists(), "its own url removes it");
    }
}
