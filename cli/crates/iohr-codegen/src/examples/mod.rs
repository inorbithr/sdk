//! Examples: for each operation of the public surface, a short program per language that
//! calls it with the published runtime, as the API reference shows it beside the
//! operation (`iohr sdk examples`).
//!
//! Every snippet is rendered from the same [`context::Op`](crate::context::Op) and the
//! same [`Naming`](crate::Naming) as the surface itself, so it names the generated
//! methods, parameter types and models exactly; the compile tests build every snippet
//! against each runtime, which is the guard against drift. A snippet:
//!
//! - builds the client as [`client`] says, the one place per language that changes when
//!   the client configuration changes;
//! - fills the path parameters and the required query parameters with the document's
//!   `example`, `default` or first enumerated value, or a `<name>` placeholder;
//! - fills a request body with its plain fields (strings, lists of strings, and numbers
//!   or booleans the document gives a value for), never an id the caller must choose;
//! - walks every page of a list with the surface's iterator, reads a stream in a loop,
//!   and handles the API's error in the language's idiom.

pub mod client;

use std::collections::BTreeMap;

use heck::ToSnakeCase as _;
use iohr_openapi::Api;
use serde::Serialize;
use serde_json::Value;

use crate::context::{self, Op, Paging};
use crate::ir::{self, Model, Shape, Type};
use crate::language::Language;
use crate::target::RenderError;

/// At most this many body fields are filled in a snippet: enough to show the shape,
/// few enough to read at a glance.
const BODY_FIELDS: usize = 4;

/// The examples of one API, as `iohr sdk examples` writes them.
#[derive(Debug, Clone, Serialize)]
pub struct Examples {
    /// The `iohr` that wrote them (`iohr 0.1.0-alpha.7`).
    pub generator: String,
    /// `info.version` of the document.
    pub api_version: String,
    /// The hash of each profile's cut, by profile, as the lock names it.
    pub cuts: BTreeMap<String, String>,
    /// The languages, in the order the docs list them.
    pub languages: Vec<String>,
    /// One entry per operation, by the document's `operationId`.
    pub operations: BTreeMap<String, OpExamples>,
    /// Why an operation has no example (an answer the surface does not render).
    pub notes: Vec<String>,
}

/// The examples of one operation.
#[derive(Debug, Clone, Serialize)]
pub struct OpExamples {
    /// `GET /v1/me`.
    pub line: String,
    /// The snippet per language, by [`Language::as_str`].
    pub code: BTreeMap<String, String>,
}

/// What a snippet does with the answer.
#[derive(Debug, Clone)]
pub(crate) enum Flow {
    /// One call, one answer.
    Unary,
    /// Every item of every page, with the surface's iterator.
    Pages(Paging),
    /// Every event of a stream.
    Stream,
}

/// One body field to fill: its wire name, its type and the value.
#[derive(Debug, Clone)]
pub(crate) struct BodyField {
    /// The wire name.
    pub name: String,
    /// Its type, one of the plain ones [`fill`] accepts.
    pub ty: Type,
    /// The value, as JSON.
    pub value: Value,
}

/// Everything a language needs to write one snippet.
#[derive(Debug, Clone)]
pub(crate) struct Call<'a> {
    /// The operation.
    pub op: &'a Op,
    /// The tag whose handle holds it; `None` for a flat operation (`me`).
    pub tag: Option<&'a str>,
    /// The path parameters' values, in path order.
    pub path: Vec<String>,
    /// The required query parameters and their values, in document order: (wire name,
    /// type, value).
    pub query: Vec<(String, Type, Value)>,
    /// The body's model name and the fields filled in it.
    pub body: Option<(String, Vec<BodyField>)>,
    /// What happens with the answer.
    pub flow: Flow,
    /// The models, for a language that needs a field's exact shape.
    pub models: &'a [Model],
}

impl Call<'_> {
    /// The name of the loop variable: the item's model in `snake_case` (`digest`,
    /// `team_event`), `event` for a stream, `item` otherwise.
    pub(crate) fn item(&self) -> String {
        match &self.flow {
            Flow::Stream => "event".into(),
            Flow::Pages(p) => match &p.item {
                Type::Ref { name } => name.to_snake_case(),
                _ => "item".into(),
            },
            Flow::Unary => "value".into(),
        }
    }

    /// The model of `name`, if it is one.
    pub(crate) fn model(&self, name: &str) -> Option<&Model> {
        self.models.iter().find(|m| m.name == name)
    }
}

/// Renders the examples of `api` in `langs`.
///
/// # Errors
///
/// [`RenderError::NotBuilt`] for a language this `iohr` does not render.
pub fn render(api: &Api, langs: &[Language], generator: &str) -> Result<Examples, RenderError> {
    if let Some(lang) = langs.iter().find(|l| !l.is_built()) {
        return Err(RenderError::NotBuilt(*lang));
    }
    let surface = context::surface(api);
    let models = context::models(api);
    let ids: BTreeMap<String, &iohr_openapi::Operation> = api
        .operations
        .values()
        .map(|o| (o.line(), o))
        .collect();
    let mut operations = BTreeMap::new();
    let handled = surface
        .handles
        .iter()
        .flat_map(|h| h.ops.iter().map(move |op| (Some(h.tag.as_str()), op)));
    let flat = surface.flat.iter().map(|op| (None, op));
    for (tag, op) in handled.chain(flat) {
        let Some(source) = ids.get(&op.line) else {
            continue;
        };
        let call = call(op, tag, source, api, &models);
        let code = langs
            .iter()
            .map(|lang| (lang.as_str().to_owned(), snippet(*lang, &call)))
            .collect();
        operations.insert(
            source.operation_id.clone(),
            OpExamples {
                line: op.line.clone(),
                code,
            },
        );
    }
    Ok(Examples {
        generator: generator.to_owned(),
        api_version: api.api_version.clone(),
        cuts: api
            .profiles
            .iter()
            .map(|(name, cut)| (name.clone(), cut.hash.clone()))
            .collect(),
        languages: langs.iter().map(|l| l.as_str().to_owned()).collect(),
        operations,
        notes: surface.notes,
    })
}

impl Examples {
    /// The examples as pretty JSON with a final newline, byte-identical for the same
    /// input.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).unwrap_or_default();
        text.push('\n');
        text
    }
}

fn snippet(lang: Language, call: &Call<'_>) -> String {
    match lang {
        Language::TypeScript => crate::typescript::example(call),
        Language::Python => crate::python::example(call),
        Language::Go => crate::go::example(call),
        Language::Java => crate::java::example(call),
        Language::CSharp => crate::csharp::example(call),
        Language::Rust => crate::rust::example(call),
    }
}

fn call<'a>(
    op: &'a Op,
    tag: Option<&'a str>,
    source: &iohr_openapi::Operation,
    api: &Api,
    models: &'a [Model],
) -> Call<'a> {
    let schema_of = |wire: &str| {
        source
            .params
            .iter()
            .find(|p| p.name == wire)
            .map(|p| &p.schema)
    };
    let path = op
        .path_params
        .iter()
        .map(|p| match schema_of(&p.name).and_then(sample) {
            Some(Value::String(s)) => s,
            Some(v) => v.to_string(),
            None => placeholder(&p.name),
        })
        .collect();
    let query = op
        .query
        .iter()
        .filter(|q| q.required)
        .map(|q| {
            let value = schema_of(&q.name)
                .and_then(sample)
                .unwrap_or_else(|| fallback(&q.name, &q.ty));
            (q.name.clone(), q.ty.clone(), value)
        })
        .collect();
    let body = op.body.as_ref().map(|name| {
        let bound: Vec<&str> = op.path_params.iter().map(|p| p.name.as_str()).collect();
        let fields = match models.iter().find(|m| &m.name == name).map(|m| &m.shape) {
            Some(Shape::Object { fields }) => fields
                .iter()
                .filter(|f| !bound.contains(&f.name.as_str()))
                .filter(|f| f.required || !f.name.ends_with("_id"))
                .filter_map(|f| {
                    let schema = api
                        .schemas
                        .get(name)
                        .and_then(|s| s.get("properties"))
                        .and_then(|p| p.get(&f.name));
                    fill(&f.name, &f.ty, schema).map(|value| BodyField {
                        name: f.name.clone(),
                        ty: f.ty.clone(),
                        value,
                    })
                })
                .take(BODY_FIELDS)
                .collect(),
            _ => Vec::new(),
        };
        (name.clone(), fields)
    });
    let flow = if op.stream {
        Flow::Stream
    } else if op.body.is_none() {
        context::paging(op, models).map_or(Flow::Unary, Flow::Pages)
    } else {
        Flow::Unary
    };
    Call {
        op,
        tag,
        path,
        query,
        body,
        flow,
        models,
    }
}

/// `<org_id>`: a value the reader replaces.
fn placeholder(name: &str) -> String {
    format!("<{name}>")
}

/// The document's own value for a schema: `example`, the first of `examples`, `default`,
/// or the first enumerated value.
fn sample(schema: &Value) -> Option<Value> {
    schema
        .get("example")
        .or_else(|| {
            schema
                .get("examples")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
        })
        .or_else(|| schema.get("default"))
        .or_else(|| {
            schema
                .get("enum")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
        })
        .cloned()
}

/// A required value with nothing in the document: a placeholder string, zero, `false`.
fn fallback(name: &str, ty: &Type) -> Value {
    match ty {
        Type::Integer { .. } | Type::Number => Value::from(0),
        Type::Bool => Value::Bool(false),
        Type::Array { .. } => Value::Array(vec![Value::String(placeholder(name))]),
        _ => Value::String(placeholder(name)),
    }
}

/// The value a body field is filled with, when it is plain enough to show: a string
/// (the document's value or a placeholder), a list of strings, and a number or a boolean
/// only when the document gives one (a guessed number or flag would change what the call
/// does). Messages, maps, enumerations and 64-bit integers are left out.
fn fill(name: &str, ty: &Type, schema: Option<&Value>) -> Option<Value> {
    let given = schema.and_then(sample);
    match ty {
        Type::String => Some(
            given
                .filter(Value::is_string)
                .unwrap_or_else(|| Value::String(placeholder(name))),
        ),
        Type::Array { item } if **item == Type::String => Some(Value::Array(vec![Value::String(
            placeholder(name),
        )])),
        Type::Integer { bits: 32 } => given.filter(Value::is_i64),
        Type::Bool => given.filter(Value::is_boolean),
        _ => None,
    }
}

/// A JSON string as a double-quoted literal, the escapes every target language shares
/// (`\"`, `\\`, `\n`).
pub(crate) fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A scalar value as a literal: a quoted string, a number, `true` or `false`.
pub(crate) fn scalar(value: &Value) -> String {
    match value {
        Value::String(s) => quoted(s),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => quoted(&other.to_string()),
    }
}

/// The strings of a list value.
pub(crate) fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|a| a.iter().map(scalar).collect())
        .unwrap_or_default()
}

/// Whether `ty` is an IR string type (a plain string, or an enumeration carried as one).
pub(crate) fn is_string(ty: &Type) -> bool {
    matches!(ty, Type::String | Type::Enum { .. })
}

/// The models the IR reads, for a target that needs `ir` itself.
pub(crate) fn object_fields<'m>(models: &'m [Model], name: &str) -> &'m [ir::Field] {
    match models.iter().find(|m| m.name == name).map(|m| &m.shape) {
        Some(Shape::Object { fields }) => fields,
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{fill, quoted, sample};
    use crate::ir::Type;

    #[test]
    fn the_document_value_comes_first() {
        assert_eq!(sample(&json!({"example": "acc_1"})), Some(json!("acc_1")));
        assert_eq!(sample(&json!({"enum": ["a", "b"]})), Some(json!("a")));
        assert_eq!(sample(&json!({"type": "string"})), None);
    }

    #[test]
    fn a_body_never_gets_a_guessed_number_or_flag() {
        assert_eq!(
            fill("url", &Type::String, None),
            Some(json!("<url>"))
        );
        assert_eq!(fill("interval_secs", &Type::Integer { bits: 32 }, None), None);
        assert_eq!(
            fill("interval_secs", &Type::Integer { bits: 32 }, Some(&json!({"default": 60}))),
            Some(json!(60))
        );
        assert_eq!(fill("paused", &Type::Bool, None), None);
        assert_eq!(quoted("a\"b"), "\"a\\\"b\"");
    }
}
