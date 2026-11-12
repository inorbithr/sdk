//! The account's incident settings as the panel's Incidents view uses them: the one setting it
//! shows and changes is `auto_resolve`, whether an incident InOrbit opened from a monitor
//! alert is resolved when every monitor on it is up again. It is the platform's own setting
//! (`incidents.proto`, `GetIncidentSettings` and `UpdateIncidentSettings`), the one the
//! console shows too; nothing is kept here.
//!
//! A platform that does not serve `auto_resolve` yet answers without the field. That is an
//! error here, so the panel says "unavailable" and never shows a switch it cannot flip.

use inorbithr::Method;
use serde_json::{Value, json};

use super::asks::{scoped, service};
use super::str_of;
use crate::Env;
use crate::cli::Global;
use crate::context::quiet_session;
use crate::error::Error;
use crate::runner::service::{HttpService, ServiceError};

/// What the panel keeps of the incident settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelIncidents {
    /// Whether auto-resolution is on for the account (the default when it never chose).
    pub(crate) auto_resolve: bool,
    /// Who chose last and when (RFC 3339); empty while the platform's default holds.
    pub(crate) updated_by: String,
    pub(crate) updated_at: String,
}

const NOT_SERVED: &str = "this platform does not serve auto-resolve yet";

/// `GetIncidentSettings`'s answer as the panel keeps it; an answer without `auto_resolve` is
/// a platform that has not got it yet.
pub(crate) fn incidents_of(v: &Value) -> Result<PanelIncidents, Error> {
    let s = v.get("settings").unwrap_or(&Value::Null);
    let auto_resolve = s
        .get("auto_resolve")
        .and_then(Value::as_bool)
        .ok_or_else(|| Error::Failed(NOT_SERVED.into()))?;
    Ok(PanelIncidents {
        auto_resolve,
        updated_by: str_of(s, "updated_by").to_owned(),
        updated_at: str_of(s, "updated_at").to_owned(),
    })
}

fn path(account: &str) -> String {
    format!("/v1/accounts/orgs/{account}/incident-settings")
}

impl HttpService<'_> {
    /// `GetIncidentSettings`: the answer as the service gives it.
    async fn incident_settings(&self) -> Result<Value, ServiceError> {
        Ok(self.api.get(&path(&self.account), &[]).await?)
    }

    /// `UpdateIncidentSettings` with `auto_resolve` alone: the other settings keep their
    /// values. Answers the settings as they are now.
    async fn set_incident_auto_resolve(&self, on: bool) -> Result<Value, ServiceError> {
        let body = json!({"settings": {"auto_resolve": on}});
        let raw = self
            .api
            .send_body(Method::Put, &path(&self.account), &body)
            .await?;
        Ok(raw.json()?)
    }
}

/// Reads the account's incident settings.
pub(crate) async fn read(g: &Global, env: &Env) -> Result<PanelIncidents, Error> {
    let s = quiet_session(g, env).await?;
    let svc = service(&s, None, None)?;
    incidents_of(&svc.incident_settings().await.map_err(scoped)?)
}

/// Turns auto-resolution on or off; the platform records who changed it and when. An older
/// platform ignores the field and answers 200, so only an answer that carries the new value
/// is a change.
pub(crate) async fn set_auto_resolve(g: &Global, env: &Env, on: bool) -> Result<(), Error> {
    let s = quiet_session(g, env).await?;
    let svc = service(&s, None, None)?;
    let v = svc.set_incident_auto_resolve(on).await.map_err(scoped)?;
    if incidents_of(&v)?.auto_resolve == on {
        Ok(())
    } else {
        Err(Error::Failed("the platform did not keep the change".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_without_the_field_is_a_platform_that_has_not_got_it() {
        let old = json!({"settings": {"open_on_alert": true, "origin": "default"}});
        let e = incidents_of(&old).unwrap_err().to_string();
        assert!(e.contains("does not serve auto-resolve"), "{e}");
        let new = json!({"settings": {"auto_resolve": true, "updated_by": "usr_1",
            "updated_at": "2026-11-10T09:00:00Z"}});
        let p = incidents_of(&new).unwrap();
        assert!(p.auto_resolve);
        assert_eq!(p.updated_by, "usr_1");
    }
}
