//! `iohr browser install`: InOrbit Trails, the browser extension (RFC 0055.1).
//!
//! The extension installs from each browser's own store, and which store fits depends on
//! the browser. The console's install page knows both: it reads the visitor's browser and
//! the live store listings (the platform's shared facts), and shows "Add to <browser>"
//! for a live listing or the load-unpacked steps before one. This command opens that page
//! in the default browser, so the command line never keeps a second list of stores.

use crate::browser;
use crate::cli::{BrowserInstall, Global};
use crate::output::Out;

/// The page that installs the extension, on the console next to the API in `base_url`.
pub(crate) fn install_url(base_url: &str) -> String {
    format!("{}/trails/#install", console_origin(base_url))
}

/// The console's origin for an API address: `https://api.<domain>` gives
/// `https://console.<domain>`; any other address gives InOrbit's own console.
fn console_origin(base_url: &str) -> String {
    url::Url::parse(base_url)
        .ok()
        .and_then(|u| {
            let host = u.host_str()?.strip_prefix("api.")?.to_owned();
            let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
            Some(format!("{}://console.{host}{port}", u.scheme()))
        })
        .unwrap_or_else(|| "https://console.inorbit.hr".to_owned())
}

pub(crate) fn install(g: &Global, args: &BrowserInstall, out: Out) {
    let url = install_url(&g.base_url);
    let opened = !args.print && browser::available() && browser::open(&url);
    if out.json {
        Out::print_json(&serde_json::json!({ "url": url, "opened": opened }));
    } else if opened {
        Out::raw(
            format!(
                "Opened {url}\nIt adds InOrbit Trails from your browser's store, or shows the install steps while that store has no listing yet.\n"
            )
            .as_bytes(),
        );
    } else {
        Out::raw(format!("Open this page in the browser you want Trails in:\n{url}\n").as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::install_url;

    #[test]
    fn the_install_page_sits_on_the_console_next_to_the_api() {
        assert_eq!(
            install_url("https://api.inorbit.hr"),
            "https://console.inorbit.hr/trails/#install"
        );
        assert_eq!(
            install_url("https://api.inorbit.hr/"),
            "https://console.inorbit.hr/trails/#install"
        );
        assert_eq!(
            install_url("http://api.localhost:18080"),
            "http://console.localhost:18080/trails/#install"
        );
    }

    #[test]
    fn an_address_without_api_falls_back_to_inorbits_console() {
        assert_eq!(
            install_url("http://127.0.0.1:9000"),
            "https://console.inorbit.hr/trails/#install"
        );
        assert_eq!(
            install_url("not a url"),
            "https://console.inorbit.hr/trails/#install"
        );
    }
}
