//! What every target renders, before any language is involved: the profiles, one
//! marker per operation with the profiles that hold it, the operations grouped by tag
//! (a handle per tag) or flat (an id without a service), each with its path split into
//! literal and parameter segments, its query parameters and body and answer types.
//! A target maps names through its [`Naming`] and types through
//! [`ir::Type`](crate::ir::Type), and renders.

use std::collections::BTreeMap;

use heck::{ToLowerCamelCase as _, ToSnakeCase as _, ToUpperCamelCase as _};
use iohr_openapi::model::env_name;
use iohr_openapi::{Api, Media, Method, Operation, ParamIn};
use serde::Serialize;

use crate::ir::{self, Type};

/// How a language spells names. The defaults are `UpperCamel` types, `snake_case`
/// methods and fields, and a trailing underscore on a reserved word; a target
/// overrides what its language does differently.
pub trait Naming {
    /// Words the language reserves, compared after casing.
    const RESERVED: &'static [&'static str];

    /// Escapes a reserved word (`type` becomes `type_`, or `r#type` in Rust).
    fn escape(&self, ident: String) -> String {
        if Self::RESERVED.contains(&ident.as_str()) {
            format!("{ident}_")
        } else {
            ident
        }
    }

    /// A type's name (`Digest`, the profile `acme-ci` as `AcmeCi`, the tag `radar` as
    /// `Radar`).
    fn type_name(&self, raw: &str) -> String {
        raw.to_upper_camel_case()
    }

    /// A method's name, from the model's `snake_case` (`get_me`).
    fn method_name(&self, snake: &str) -> String {
        self.escape(snake.to_owned())
    }

    /// A field's or parameter's name, from its wire name (`org_id`).
    fn field_name(&self, wire: &str) -> String {
        self.escape(wire.to_snake_case())
    }
}

/// `lowerCamel` methods and fields, for the languages that spell them so.
#[must_use]
pub fn lower_camel(raw: &str) -> String {
    raw.to_lower_camel_case()
}

/// The whole surface, language-neutral.
#[derive(Debug, Clone, Serialize)]
pub struct Surface {
    /// The line every generated file starts with ([`crate::header`]).
    pub header: String,
    /// `info.version` of the documents.
    pub api_version: String,
    /// The profiles, in name order.
    pub profiles: Vec<Profile>,
    /// One marker per rendered operation, in `METHOD /path` order.
    pub markers: Vec<Marker>,
    /// The operations whose id names a service, one handle per tag, in tag order.
    pub handles: Vec<Handle>,
    /// The operations whose id names no service (`me`), in `METHOD /path` order.
    pub flat: Vec<Op>,
    /// Why an operation was left out (an answer that is neither JSON nor a stream).
    pub notes: Vec<String>,
}

/// One profile.
#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    /// The profile's name (`acme-ci`).
    pub name: String,
    /// What its environment variables carry (`ACME_CI` in `INORBIT_ACME_CI_TOKEN`).
    pub env: String,
    /// `true` for `public`, the runtime's own profile.
    pub is_public: bool,
}

/// The marker of one operation, and the profiles whose cut holds it.
#[derive(Debug, Clone, Serialize)]
pub struct Marker {
    /// `UpperCamel`, after the operation; two operations of one name in different tags
    /// both take their tag as a prefix (`EventsListEndpoints`).
    pub name: String,
    /// `GET /v1/me`.
    pub line: String,
    /// The profile names, sorted.
    pub profiles: Vec<String>,
}

/// One tag's operations.
#[derive(Debug, Clone, Serialize)]
pub struct Handle {
    /// The tag (`accounts`).
    pub tag: String,
    /// Its operations, in `METHOD /path` order.
    pub ops: Vec<Op>,
}

/// One part of a path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Segment {
    /// Literal text, sent as is.
    Literal {
        /// The text, slashes included.
        text: String,
    },
    /// A path parameter, percent-encoded as one segment.
    Param {
        /// The wire name.
        name: String,
    },
}

/// A path or query parameter.
#[derive(Debug, Clone, Serialize)]
pub struct Param {
    /// The wire name.
    pub name: String,
    /// Its type.
    pub ty: Type,
    /// Whether a call must give it (always, for a path parameter).
    pub required: bool,
    /// Its description, if any.
    pub doc: String,
}

/// One operation, as every target renders it.
#[derive(Debug, Clone, Serialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent facts about one operation that every target reads by name"
)]
pub struct Op {
    /// The model's `snake_case` name (`get_me`).
    pub name: String,
    /// What hooks see: `accounts.get_me`, or `me` for a flat operation.
    pub hook_name: String,
    /// The marker's name.
    pub marker: String,
    /// The first paragraph of the description.
    pub doc: String,
    /// `GET /v1/me`.
    pub line: String,
    /// The method.
    pub method: Method,
    /// The path template as the document has it.
    pub path: String,
    /// The path split into literal and parameter parts.
    pub segments: Vec<Segment>,
    /// The path parameters, in path order.
    pub path_params: Vec<Param>,
    /// The query parameters, in document order.
    pub query: Vec<Param>,
    /// The name of the type that carries the query parameters
    /// (`AccountsGetUsageParams`, or `GetUsageParams` for a flat operation), when there
    /// are any.
    pub params_type: Option<String>,
    /// The request body's model name.
    pub body: Option<String>,
    /// The answer's model name; `None` when the answer is not one model.
    pub response: Option<String>,
    /// The scopes the operation needs.
    pub scopes: Vec<String>,
    /// Whether the call may be retried: the method is idempotent, or the operation says
    /// so.
    pub retry_safe: bool,
    /// Whether the operation is marked idempotent although its method is not, which the
    /// runtime must be told.
    pub idempotent_override: bool,
    /// Whether the operation takes an `Idempotency-Key` header, which the runtime's
    /// `idempotency_key` middleware generates once per call and sends on every attempt,
    /// so the write may be retried (`docs/config.md` section 7.5). A target passes it to
    /// the runtime's operation as it does [`idempotent_override`](Self::idempotent_override)
    /// once the runtime has the field (M6).
    pub idempotency_key: bool,
    /// The profiles that may call it.
    pub profiles: Vec<String>,
    /// Whether the answer is a stream (`text/event-stream`): the method yields the
    /// `response` model once per event instead of answering once (design.md section 7).
    pub stream: bool,
    /// The RPC's full name a `/v1/ws` call frame names (`x-iohr-rpc`); empty when the
    /// document gives none, and then the stream opens over server-sent events only.
    pub rpc: String,
}

/// How a list operation pages, by wire names: the query parameter that carries the
/// token, the answer's list and next-token fields, and the type of one item.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Paging {
    /// The query parameter's wire name (`page_token`).
    pub token_param: String,
    /// The index of that parameter in [`Op::query`].
    pub token_index: usize,
    /// The answer's list field, by wire name.
    pub list_field: String,
    /// The answer's next-token field, by wire name (`next_page_token`).
    pub next_field: String,
    /// The type of one item of the list.
    pub item: Type,
}

/// The paging of `op`, when it pages: a string `page_token` query parameter, and an answer
/// with a string `next_page_token` and exactly one list. The rule every target's
/// `All<Op>` iterator follows (design.md §9).
#[must_use]
pub fn paging(op: &Op, models: &[ir::Model]) -> Option<Paging> {
    const TOKEN: [&str; 2] = ["page_token", "pageToken"];
    const NEXT: [&str; 2] = ["next_page_token", "nextPageToken"];
    let token_index = op
        .query
        .iter()
        .position(|q| TOKEN.contains(&q.name.as_str()) && q.ty == Type::String)?;
    let response = op.response.as_deref()?;
    let ir::Shape::Object { fields } = &models.iter().find(|m| m.name == response)?.shape else {
        return None;
    };
    let next = fields
        .iter()
        .find(|f| NEXT.contains(&f.name.as_str()) && f.ty == Type::String)?;
    let mut lists = fields.iter().filter(|f| matches!(f.ty, Type::Array { .. }));
    let list = lists.next()?;
    if lists.next().is_some() {
        return None;
    }
    let Type::Array { item } = &list.ty else {
        return None;
    };
    Some(Paging {
        token_param: op.query[token_index].name.clone(),
        token_index,
        list_field: list.name.clone(),
        next_field: next.name.clone(),
        item: (**item).clone(),
    })
}

/// The surface of `api`: every operation whose answer is JSON or a stream of JSON
/// events; another media type is left out with a note.
#[must_use]
pub fn surface(api: &Api) -> Surface {
    let markers = marker_names(api);
    let mut notes = Vec::new();
    let mut by_tag: BTreeMap<String, Vec<Op>> = BTreeMap::new();
    let mut flat = Vec::new();
    let mut marker_list = Vec::new();
    for op in api.operations.values() {
        match &op.response.media {
            Media::Json => {}
            Media::EventStream if op.response.schema.is_some() => {}
            Media::EventStream => {
                notes.push(format!(
                    "{}: a stream whose events name no schema; left out",
                    op.line()
                ));
                continue;
            }
            Media::Other(m) => {
                notes.push(format!(
                    "{}: answers {m}, which is not rendered; left out",
                    op.line()
                ));
                continue;
            }
        }
        let marker = markers[&op.line()].clone();
        marker_list.push(Marker {
            name: marker.clone(),
            line: op.line(),
            profiles: op.profiles.iter().cloned().collect(),
        });
        let ctx = op_context(op, marker);
        if op.service.is_some() {
            by_tag.entry(op.tag.clone()).or_default().push(ctx);
        } else {
            flat.push(ctx);
        }
    }
    Surface {
        header: crate::header(api),
        api_version: api.api_version.clone(),
        profiles: api
            .profiles
            .keys()
            .map(|name| Profile {
                name: name.clone(),
                env: env_name(name),
                is_public: name == "public",
            })
            .collect(),
        markers: marker_list,
        handles: by_tag
            .into_iter()
            .map(|(tag, ops)| Handle { tag, ops })
            .collect(),
        flat,
        notes,
    }
}

/// The models of `api`, through the IR.
#[must_use]
pub fn models(api: &Api) -> Vec<ir::Model> {
    ir::models(&api.schemas)
}

/// One marker per operation, named after the operation; when two tags share a name,
/// both take their tag as a prefix.
fn marker_names(api: &Api) -> BTreeMap<String, String> {
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    for op in api.operations.values() {
        *count.entry(op.name.to_upper_camel_case()).or_default() += 1;
    }
    api.operations
        .values()
        .map(|op| {
            let short = op.name.to_upper_camel_case();
            let name = if count[&short] > 1 {
                format!("{}{short}", op.tag.to_upper_camel_case())
            } else {
                short
            };
            (op.line(), name)
        })
        .collect()
}

fn op_context(op: &Operation, marker: String) -> Op {
    let param = |p: &iohr_openapi::Param| Param {
        name: p.name.clone(),
        ty: ir::type_of(&p.schema),
        required: p.required || p.location == ParamIn::Path,
        doc: p.doc.clone(),
    };
    let path_params: Vec<Param> = op
        .params
        .iter()
        .filter(|p| p.location == ParamIn::Path)
        .map(param)
        .collect();
    let query: Vec<Param> = op
        .params
        .iter()
        .filter(|p| p.location == ParamIn::Query)
        .map(param)
        .collect();
    let params_type = (!query.is_empty()).then(|| {
        let base = format!("{}Params", op.name.to_upper_camel_case());
        match &op.service {
            Some(_) => format!("{}{base}", op.tag.to_upper_camel_case()),
            None => base,
        }
    });
    Op {
        name: op.name.clone(),
        hook_name: match &op.service {
            Some(_) => format!("{}.{}", op.tag, op.name),
            None => op.name.clone(),
        },
        marker,
        doc: op.doc.clone(),
        line: op.line(),
        method: op.method,
        path: op.path.clone(),
        segments: segments(&op.path),
        path_params,
        query,
        params_type,
        body: op.request_body.clone(),
        response: op.response.schema.clone(),
        scopes: op.scopes.clone(),
        retry_safe: op.idempotent || op.method.is_idempotent(),
        idempotent_override: op.idempotent && !op.method.is_idempotent(),
        idempotency_key: op.idempotency_key,
        profiles: op.profiles.iter().cloned().collect(),
        stream: op.response.media == Media::EventStream,
        rpc: op.rpc.clone().unwrap_or_default(),
    }
}

/// `/v1/orgs/{org_id}/usage` as `[/v1/orgs/, {org_id}, /usage]`.
#[must_use]
pub fn segments(path: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        if open > 0 {
            out.push(Segment::Literal {
                text: rest[..open].to_owned(),
            });
        }
        out.push(Segment::Param {
            name: rest[open + 1..open + close].to_owned(),
        });
        rest = &rest[open + close + 1..];
    }
    if !rest.is_empty() {
        out.push(Segment::Literal {
            text: rest.to_owned(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Naming, Segment, segments};

    struct Plain;
    impl Naming for Plain {
        const RESERVED: &'static [&'static str] = &["type"];
    }

    #[test]
    fn a_path_splits_into_literals_and_parameters() {
        assert_eq!(
            segments("/v1/orgs/{org_id}/keys/{key_id}"),
            [
                Segment::Literal {
                    text: "/v1/orgs/".into()
                },
                Segment::Param {
                    name: "org_id".into()
                },
                Segment::Literal {
                    text: "/keys/".into()
                },
                Segment::Param {
                    name: "key_id".into()
                },
            ]
        );
        assert_eq!(
            segments("/v1/me"),
            [Segment::Literal {
                text: "/v1/me".into()
            }]
        );
    }

    #[test]
    fn naming_defaults_escape_reserved_words() {
        assert_eq!(Plain.field_name("type"), "type_");
        assert_eq!(Plain.field_name("orgId"), "org_id");
        assert_eq!(Plain.type_name("acme-ci"), "AcmeCi");
        assert_eq!(super::lower_camel("get_usage"), "getUsage");
    }
}
