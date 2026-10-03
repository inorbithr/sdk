//! A hook point around every attempt (`docs/design.md` section 8): enough to add
//! logging, metrics or tracing without the SDK depending on any of them.

use crate::Method;
use crate::error::{Error, RawResponse};

/// One attempt of one call, as the hooks see it: metadata only, never a header, a
/// query value or a body (SR-13).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Attempt {
    /// The operation's name (`accounts.get_me`), or `request` for a raw call.
    pub operation: &'static str,
    /// The method.
    pub method: Method,
    /// The path, with its parameters bound, without the query.
    pub path: String,
    /// 1 for the first attempt.
    pub number: u32,
    /// The client request id (`x-request-id`).
    pub request_id: String,
}

/// Observes attempts. Every method has an empty default, so a hook implements only
/// what it needs.
pub trait Hook: Send + Sync {
    /// Called just before an attempt is sent.
    fn on_request(&self, attempt: &Attempt) {
        let _ = attempt;
    }

    /// Called when an attempt got an answer, whatever its status.
    fn on_response(&self, attempt: &Attempt, response: &RawResponse) {
        let _ = (attempt, response);
    }

    /// Called when an attempt failed before an answer, or when a call gives up.
    fn on_error(&self, attempt: &Attempt, error: &Error) {
        let _ = (attempt, error);
    }
}
