//! One or several normalised documents as one API: the operations, which profiles may
//! call each, the schemas they use, and the cut each profile's document carries.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use heck::{ToSnakeCase as _, ToUpperCamelCase as _};
use serde_json::Value;

use crate::error::ModelError;
use crate::hash::cut_hash;
use crate::normalise::{Normalised, is_method, normalise};

/// An HTTP method of an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    /// `GET`
    Get,
    /// `POST`
    Post,
    /// `PUT`
    Put,
    /// `PATCH`
    Patch,
    /// `DELETE`
    Delete,
    /// `HEAD`
    Head,
}

impl Method {
    fn parse(key: &str) -> Option<Self> {
        Some(match key {
            "get" => Self::Get,
            "post" => Self::Post,
            "put" => Self::Put,
            "patch" => Self::Patch,
            "delete" => Self::Delete,
            "head" => Self::Head,
            _ => return None,
        })
    }

    /// `GET`, `POST`, ...
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
        }
    }

    /// Whether the method is idempotent by definition.
    #[must_use]
    pub fn is_idempotent(self) -> bool {
        matches!(self, Self::Get | Self::Put | Self::Delete | Self::Head)
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a profile's document was cut to: `info.x-iohr-cut` as the gateway stamps it,
/// or computed here when the document carries no stamp.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Cut {
    /// The plan the document was cut by (`free`, `public`).
    #[serde(default)]
    pub plan: String,
    /// The account the document is for, when the gateway said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The scopes it was narrowed to, sorted; empty for a person or the public document.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// `sha256:<hex>` over the projection `hash::cut_hash` describes.
    #[serde(default)]
    pub hash: String,
    /// `true` when the hash was computed here because the document had no stamp.
    #[serde(default, skip_serializing)]
    pub local: bool,
}

impl Cut {
    fn read(doc: &Value) -> Self {
        let stamped: Option<Cut> = doc
            .pointer("/info/x-iohr-cut")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        match stamped {
            Some(mut c) if !c.hash.is_empty() => {
                c.scopes.sort();
                c.local = false;
                c
            }
            other => Self {
                plan: other
                    .map(|c| c.plan)
                    .filter(|p| !p.is_empty())
                    .unwrap_or_else(|| "public".into()),
                account: None,
                scopes: Vec::new(),
                hash: cut_hash(doc),
                local: true,
            },
        }
    }
}

/// Where a parameter goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamIn {
    /// In the path, always required.
    Path,
    /// In the query string.
    Query,
}

/// One parameter of an operation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Param {
    /// The wire name (`org_id`).
    pub name: String,
    /// Where it goes.
    pub location: ParamIn,
    /// Whether a call must give it.
    pub required: bool,
    /// Its JSON Schema, as the document has it.
    pub schema: Value,
    /// Its description, if any.
    pub doc: String,
}

/// The media type of a successful answer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Media {
    /// `application/json`
    Json,
    /// `text/event-stream`: a stream, which the targets do not generate yet.
    EventStream,
    /// Something else, named.
    Other(String),
}

/// The `200` answer of an operation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Response {
    /// The schema's short name when the answer is one `$ref`.
    pub schema: Option<String>,
    /// The media type.
    pub media: Media,
}

/// One operation and who may call it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Operation {
    /// The method name in the generated surface, `snake_case` (`get_me`).
    pub name: String,
    /// The operation id as the document has it (`AccountsService.GetMe`, `me`).
    pub operation_id: String,
    /// The service the id names (`AccountsService`), if any.
    pub service: Option<String>,
    /// The tag the operation is grouped under (`accounts`); a handle per tag.
    pub tag: String,
    /// The method.
    pub method: Method,
    /// The path template (`/v1/accounts/orgs/{org_id}/usage`).
    pub path: String,
    /// Path and query parameters, in document order.
    pub params: Vec<Param>,
    /// The request body's schema name, for a `POST`/`PUT`/`PATCH` with one.
    pub request_body: Option<String>,
    /// The `200` answer.
    pub response: Response,
    /// The scopes the operation needs (`x-iohr-scopes`).
    pub scopes: Vec<String>,
    /// The profiles whose document holds the operation.
    pub profiles: BTreeSet<String>,
    /// Whether the generated method may be retried without a key.
    pub idempotent: bool,
    /// The description, first paragraph.
    pub doc: String,
}

impl Operation {
    /// `GET /v1/me`: the line the lock file keeps per operation.
    #[must_use]
    pub fn line(&self) -> String {
        format!("{} {}", self.method, self.path)
    }
}

/// Several documents, one API: the union of their operations, each tagged with the
/// profiles that may call it, and the schemas they share.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Api {
    /// The cut of each profile's document, by profile name.
    pub profiles: BTreeMap<String, Cut>,
    /// Every operation, by `METHOD /path`.
    pub operations: BTreeMap<String, Operation>,
    /// Every schema any kept operation reaches, by short name.
    pub schemas: BTreeMap<String, Value>,
    /// The error envelope as a JSON Schema (`spec/problem.json`).
    pub problem: Value,
    /// `info.version` of the documents.
    pub api_version: String,
}

impl Api {
    /// Models `docs`, one `(profile, document)` each, as one API. Each document is
    /// normalised first.
    ///
    /// # Errors
    ///
    /// [`ModelError`] when a document cannot be normalised, holds no operation, two
    /// documents disagree on a schema or an operation, two profile names would read the
    /// same environment variables, or no document was given.
    pub fn from_documents(
        docs: impl IntoIterator<Item = (String, Value)>,
    ) -> Result<Self, ModelError> {
        let mut api = Self {
            profiles: BTreeMap::new(),
            operations: BTreeMap::new(),
            schemas: BTreeMap::new(),
            problem: Value::Null,
            api_version: String::new(),
        };
        let mut env_names: BTreeMap<String, String> = BTreeMap::new();
        for (profile, raw) in docs {
            let env = env_name(&profile);
            if let Some(other) = env_names.insert(env.clone(), profile.clone())
                && other != profile
            {
                return Err(ModelError::AmbiguousProfiles {
                    a: other,
                    b: profile,
                    env,
                });
            }
            let Normalised { openapi, problem } = normalise(&raw)?;
            api.add(&profile, &raw, &openapi, problem)?;
        }
        if api.profiles.is_empty() {
            return Err(ModelError::NoDocuments);
        }
        api.settle_names();
        Ok(api)
    }

    fn add(
        &mut self,
        profile: &str,
        raw: &Value,
        doc: &Value,
        problem: Value,
    ) -> Result<(), ModelError> {
        self.profiles.insert(profile.to_owned(), Cut::read(raw));
        if self.api_version.is_empty() {
            doc.pointer("/info/version")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .clone_into(&mut self.api_version);
        }
        if self.problem.is_null() {
            self.problem = problem;
        }
        let mut any = false;
        if let Some(paths) = doc.get("paths").and_then(Value::as_object) {
            for (path, item) in paths {
                let Some(item) = item.as_object() else {
                    continue;
                };
                for (key, op) in item.iter().filter(|(k, _)| is_method(k)) {
                    let Some(method) = Method::parse(key) else {
                        continue;
                    };
                    any = true;
                    let mut parsed = read_operation(method, path, op)?;
                    let line = parsed.line();
                    if let Some(existing) = self.operations.get_mut(&line) {
                        if !same_shape(existing, &parsed) {
                            return Err(ModelError::OperationConflict {
                                a: existing.profiles.iter().next().cloned().unwrap_or_default(),
                                b: profile.to_owned(),
                                method: method.to_string(),
                                path: path.clone(),
                            });
                        }
                        existing.profiles.insert(profile.to_owned());
                    } else {
                        parsed.profiles.insert(profile.to_owned());
                        self.operations.insert(line, parsed);
                    }
                }
            }
        }
        if !any {
            return Err(ModelError::NothingToCall {
                profile: profile.to_owned(),
            });
        }
        if let Some(schemas) = doc
            .pointer("/components/schemas")
            .and_then(Value::as_object)
        {
            for (name, schema) in schemas {
                match self.schemas.get(name) {
                    Some(existing) if existing != schema => {
                        let a = self
                            .profiles
                            .keys()
                            .find(|p| p.as_str() != profile)
                            .cloned()
                            .unwrap_or_default();
                        return Err(ModelError::SchemaConflict {
                            a,
                            b: profile.to_owned(),
                            schema: name.clone(),
                        });
                    }
                    Some(_) => {}
                    None => {
                        self.schemas.insert(name.clone(), schema.clone());
                    }
                }
            }
        }
        Ok(())
    }

    /// Two operations in different tags may share a short name; the later one in
    /// `METHOD /path` order takes its tag as a prefix.
    fn settle_names(&mut self) {
        let mut seen: BTreeMap<(String, String), String> = BTreeMap::new();
        let lines: Vec<String> = self.operations.keys().cloned().collect();
        for line in lines {
            let op = &self.operations[&line];
            let key = (op.tag.clone(), op.name.clone());
            if let Some(first) = seen.get(&key)
                && first != &line
            {
                let renamed = format!("{}_{}", op.name, op.method.as_str().to_lowercase());
                if let Some(op) = self.operations.get_mut(&line) {
                    op.name = renamed;
                }
            } else {
                seen.insert(key, line);
            }
        }
    }

    /// The operations a profile may call, in `METHOD /path` order.
    pub fn operations_of<'a>(
        &'a self,
        profile: &'a str,
    ) -> impl Iterator<Item = &'a Operation> + 'a {
        self.operations
            .values()
            .filter(move |op| op.profiles.contains(profile))
    }

    /// The tags any operation uses, sorted.
    #[must_use]
    pub fn tags(&self) -> BTreeSet<&str> {
        self.operations.values().map(|op| op.tag.as_str()).collect()
    }
}

/// `acme-ci` reads `INORBIT_ACME_CI_*`: the name as the runtime's `env_prefix` makes it.
#[must_use]
pub fn env_name(profile: &str) -> String {
    profile
        .chars()
        .map(|c| match c {
            '-' => '_',
            c => c.to_ascii_uppercase(),
        })
        .collect()
}

/// The Rust type name of a profile (`acme-ci` becomes `AcmeCi`).
#[must_use]
pub fn type_name(profile: &str) -> String {
    profile.to_upper_camel_case()
}

fn same_shape(a: &Operation, b: &Operation) -> bool {
    a.operation_id == b.operation_id
        && a.params == b.params
        && a.request_body == b.request_body
        && a.response == b.response
        && a.scopes == b.scopes
}

fn schema_name(node: &Value) -> Option<String> {
    node.get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.rsplit('/').next())
        .map(str::to_owned)
}

fn read_params(op: &Value) -> Vec<Param> {
    op.get("parameters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let name = p.get("name")?.as_str()?.to_owned();
            let location = match p.get("in")?.as_str()? {
                "path" => ParamIn::Path,
                "query" => ParamIn::Query,
                _ => return None,
            };
            Some(Param {
                name,
                required: location == ParamIn::Path
                    || p.get("required") == Some(&Value::Bool(true)),
                location,
                schema: p.get("schema").cloned().unwrap_or(Value::Null),
                doc: p
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            })
        })
        .collect()
}

fn read_response(op: &Value) -> Response {
    let content = op
        .pointer("/responses/200/content")
        .and_then(Value::as_object);
    match content.and_then(|c| c.iter().next()) {
        Some((media, body)) => Response {
            schema: body.get("schema").and_then(schema_name),
            media: match media.as_str() {
                "application/json" => Media::Json,
                "text/event-stream" => Media::EventStream,
                other => Media::Other(other.to_owned()),
            },
        },
        None => Response {
            schema: None,
            media: Media::Json,
        },
    }
}

fn read_operation(method: Method, path: &str, op: &Value) -> Result<Operation, ModelError> {
    let operation_id = op
        .get("operationId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ModelError::NoOperationId {
            method: method.to_string(),
            path: path.to_owned(),
        })?
        .to_owned();
    let (service, short) = match operation_id.rsplit_once('.') {
        Some((s, op)) => (Some(s.to_owned()), op.to_owned()),
        None => (None, operation_id.clone()),
    };
    let tag = op
        .get("tags")
        .and_then(Value::as_array)
        .and_then(|t| t.first())
        .and_then(Value::as_str)
        .map_or_else(
            || {
                service.as_deref().map_or("default".to_owned(), |s| {
                    s.strip_suffix("Service").unwrap_or(s).to_snake_case()
                })
            },
            str::to_owned,
        );
    let scopes = op
        .get("x-iohr-scopes")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let doc = op
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .split("\n\n")
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned();
    Ok(Operation {
        name: short.to_snake_case(),
        operation_id,
        service,
        tag,
        method,
        path: path.to_owned(),
        params: read_params(op),
        request_body: op
            .pointer("/requestBody/content/application~1json/schema")
            .and_then(schema_name),
        response: read_response(op),
        scopes,
        profiles: BTreeSet::new(),
        idempotent: method.is_idempotent(),
        doc,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Api, Method, env_name, type_name};
    use crate::error::ModelError;

    fn doc(
        ops: &[(&str, &str, &str, &[&str])],
        cut: Option<serde_json::Value>,
    ) -> serde_json::Value {
        let mut paths = serde_json::Map::new();
        for (method, path, id, scopes) in ops {
            let item = paths.entry((*path).to_owned()).or_insert_with(|| json!({}));
            item[*method] = json!({
                "operationId": id, "x-iohr-public": true, "x-iohr-scopes": scopes, "tags": [id.split('.').next().unwrap().trim_end_matches("Service").to_lowercase()],
                "parameters": if path.contains('{') { json!([{"name": "org_id", "in": "path", "required": true, "schema": {"type": "string"}}]) } else { json!([]) },
                "responses": { "200": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Me" } } } },
                               "default": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Problem" } } } } }
            });
        }
        let mut info = json!({ "title": "t", "version": "0.1.0" });
        if let Some(c) = cut {
            info["x-iohr-cut"] = c;
        }
        json!({
            "openapi": "3.1.0", "info": info, "paths": paths,
            "components": { "schemas": {
                "Me": { "type": "object", "properties": { "subject": { "type": "string" } } },
                "Problem": { "type": "object", "properties": { "code": { "$ref": "#/components/schemas/Code" }, "details": { "type": "array", "items": { "$ref": "#/components/schemas/Detail" } } } },
                "Code": { "type": "string", "enum": ["bad_request","failed_precondition","unauthenticated","forbidden","not_found","method_not_allowed","already_exists","conflict","payload_too_large","unsupported_media_type","rate_limited","quota_exceeded","cancelled","internal","unimplemented","unavailable","timeout"] },
                "Detail": { "oneOf": [ { "type": "object", "required": ["type"], "properties": { "type": { "const": "retry" } } } ] }
            } }
        })
    }

    #[test]
    fn two_profiles_merge_by_operation_and_keep_who_may_call_what() {
        let personal = doc(
            &[
                ("get", "/v1/me", "me", &["identity:read"]),
                (
                    "get",
                    "/v1/radar/digests",
                    "RadarService.ListDigests",
                    &["radar:read"],
                ),
            ],
            Some(
                json!({ "plan": "free", "account": "acc_1", "scopes": ["radar:read", "identity:read"], "hash": "sha256:aa" }),
            ),
        );
        let ci = doc(
            &[
                ("get", "/v1/me", "me", &["identity:read"]),
                (
                    "get",
                    "/v1/accounts/orgs/{org_id}/usage",
                    "AccountsService.GetUsage",
                    &["usage:read"],
                ),
            ],
            None,
        );
        let api = Api::from_documents([
            ("personal".to_owned(), personal),
            ("acme-ci".to_owned(), ci),
        ])
        .unwrap();
        assert_eq!(api.operations.len(), 3);
        let me = &api.operations["GET /v1/me"];
        assert_eq!(me.name, "me");
        assert_eq!(me.tag, "me");
        assert_eq!(
            me.profiles.iter().collect::<Vec<_>>(),
            ["acme-ci", "personal"]
        );
        let digests = &api.operations["GET /v1/radar/digests"];
        assert_eq!(
            (digests.name.as_str(), digests.tag.as_str(), digests.method),
            ("list_digests", "radar", Method::Get)
        );
        assert_eq!(digests.profiles.iter().collect::<Vec<_>>(), ["personal"]);
        let usage = &api.operations["GET /v1/accounts/orgs/{org_id}/usage"];
        assert_eq!(usage.params[0].name, "org_id");
        assert_eq!(usage.response.schema.as_deref(), Some("Me"));
        assert_eq!(
            api.profiles["personal"].scopes,
            ["identity:read", "radar:read"]
        );
        assert!(!api.profiles["personal"].local);
        assert!(
            api.profiles["acme-ci"].local && api.profiles["acme-ci"].hash.starts_with("sha256:")
        );
        assert_eq!(api.api_version, "0.1.0");
        assert_eq!(
            api.tags().into_iter().collect::<Vec<_>>(),
            ["accounts", "me", "radar"]
        );
    }

    #[test]
    fn names_that_would_share_variables_and_empty_cuts_are_refused() {
        let a = doc(&[("get", "/v1/me", "me", &["identity:read"])], None);
        let err = Api::from_documents([
            ("acme-ci".to_owned(), a.clone()),
            ("acme_ci".to_owned(), a.clone()),
        ])
        .unwrap_err();
        assert!(matches!(err, ModelError::AmbiguousProfiles { .. }), "{err}");
        let mut empty = a.clone();
        empty["paths"] = json!({});
        let err = Api::from_documents([("p".to_owned(), empty)]).unwrap_err();
        assert!(matches!(err, ModelError::Normalise(_)), "{err}");
        assert!(matches!(
            Api::from_documents(Vec::new()).unwrap_err(),
            ModelError::NoDocuments
        ));
        assert_eq!(env_name("acme-ci"), "ACME_CI");
        assert_eq!(type_name("acme-ci"), "AcmeCi");
    }
}
