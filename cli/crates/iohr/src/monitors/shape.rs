//! Body shape checks (`expect.shape`, RFC 0008): the body is read at most 1 MiB, checked in
//! memory against a JSON Schema and dropped. A failure names only the first mismatch, as a
//! location and two types, never a value (core RFC 0040.2, "Reading a body, and what leaves
//! the machine").
//!
//! The module does not know where schemas live: the caller hands in [`Shapes`]. The command backs it
//! with the project's `schemas/` folder over the embedded ones (`schema::load_tree`).

use jsonschema::error::{TypeKind, ValidationErrorKind};
use jsonschema::{JsonType, Validator};
use serde_json::Value;
use std::fmt;
use std::sync::Arc;

/// The most body read for a shape check (RFC 0040.2).
pub const MAX_BODY: usize = 1024 * 1024;
/// The most events read from a stream for a shape check (RFC 0040.2).
pub const MAX_EVENTS: u32 = 20;

/// Where the schemas named by `expect.shape` come from.
pub trait Shapes {
    /// The compiled schema for a shape path such as `schemas/api/items/v0.json`.
    fn validator(&self, shape: &str) -> Result<Arc<Validator>, String>;
}

/// Any function from a shape path to its schema is a [`Shapes`], so a caller passes a closure.
impl<F: Fn(&str) -> Result<Arc<Validator>, String>> Shapes for F {
    fn validator(&self, shape: &str) -> Result<Arc<Validator>, String> {
        self(shape)
    }
}

/// The `$id` of the schema at a repo path such as `schemas/api/items/v0.json`, for schema sets
/// whose ids are a base URL plus the path under `schemas/` (a project's are). None for any other path.
pub fn schema_id(base: &str, shape: &str) -> Option<String> {
    shape
        .strip_prefix("schemas/")
        .filter(|rest| !rest.is_empty() && !rest.contains(".."))
        .map(|rest| format!("{base}{rest}"))
}

/// The first place a body breaks its schema: a JSON pointer and two type words, no value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    /// A JSON pointer into the body, `/` for the root.
    pub at: String,
    /// What the schema wants: a JSON type, `a field`, or the rule's keyword.
    pub expected: String,
    /// What the body has: a JSON type, or `none` for a missing field.
    pub found: String,
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: expected {}, found {}",
            self.at, self.expected, self.found
        )
    }
}

/// The JSON type of a value, as the schema spells it.
pub fn type_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn types(kind: &TypeKind) -> String {
    match kind {
        TypeKind::Single(t) => t.as_str().to_owned(),
        TypeKind::Multiple(set) => set
            .iter()
            .map(JsonType::as_str)
            .collect::<Vec<_>>()
            .join(" or "),
    }
}

/// The first mismatch of `value` against `schema`, or None when it matches.
///
/// Only names leave: the pointer (which holds the body's keys and indexes, as the schema
/// declares them), the schema's type or keyword, and the body's JSON type. A key the schema
/// does not allow is reported at its parent, so its name never leaves either.
pub fn first_mismatch(schema: &Validator, value: &Value) -> Option<Mismatch> {
    let e = schema.iter_errors(value).next()?;
    let at = e.instance_path().to_string();
    let at = if at.is_empty() { "/".to_owned() } else { at };
    let found = type_of(e.instance()).to_owned();
    Some(match e.kind() {
        ValidationErrorKind::Type { kind } => Mismatch {
            at,
            expected: types(kind),
            found,
        },
        ValidationErrorKind::Required { property } => {
            let field = property.as_str().unwrap_or_default();
            let at = if at == "/" {
                format!("/{field}")
            } else {
                format!("{at}/{field}")
            };
            Mismatch {
                at,
                expected: "a field".into(),
                found: "none".into(),
            }
        }
        ValidationErrorKind::AdditionalProperties { .. }
        | ValidationErrorKind::UnevaluatedProperties { .. } => Mismatch {
            at,
            expected: "no other fields".into(),
            found: "another field".into(),
        },
        k => Mismatch {
            at,
            expected: k.keyword().to_owned(),
            found,
        },
    })
}

/// Checks one JSON body against the schema for `shape`: Ok, or the line to report
/// (`<pointer>: expected X, found Y`, or why it could not be checked). The body is not kept.
pub fn check_body(shapes: &dyn Shapes, shape: &str, body: &[u8]) -> Result<(), String> {
    let schema = shapes.validator(shape)?;
    let value: Value = serde_json::from_slice(body).map_err(|e| {
        format!(
            "/: expected JSON, found a parse error at line {} column {}",
            e.line(),
            e.column()
        )
    })?;
    match first_mismatch(&schema, &value) {
        None => Ok(()),
        Some(m) => Err(m.to_string()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use serde_json::json;

    fn schema() -> Validator {
        jsonschema::validator_for(&json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["items"],
            "properties": {
                "items": { "type": "array", "items": {
                    "type": "object", "required": ["id", "score"],
                    "properties": { "id": { "type": "integer" }, "score": { "type": "integer" }, "summary": { "type": ["string", "null"] }, "kind": { "enum": ["feed", "page"] } }
                } }
            }
        }))
        .unwrap()
    }

    #[test]
    fn a_mismatch_names_the_place_and_two_types_never_the_value() {
        let s = schema();
        let ok = json!({ "items": [{ "id": 1, "score": 3, "summary": null }] });
        assert_eq!(first_mismatch(&s, &ok), None);

        let wrong = json!({ "items": [{ "id": 1, "score": 2 }, { "id": 2, "score": 1 }, { "id": 3, "score": 0 }, { "id": 4, "score": "secret-token-123" }] });
        let m = first_mismatch(&s, &wrong).unwrap();
        assert_eq!(
            m.to_string(),
            "/items/3/score: expected integer, found string"
        );
        assert!(!m.to_string().contains("secret"));

        let missing = json!({ "items": [{ "id": 1 }] });
        assert_eq!(
            first_mismatch(&s, &missing).unwrap().to_string(),
            "/items/0/score: expected a field, found none"
        );

        let extra = json!({ "items": [], "password-hunter2": true });
        let m = first_mismatch(&s, &extra).unwrap().to_string();
        assert_eq!(m, "/: expected no other fields, found another field");

        let bad_enum = json!({ "items": [{ "id": 1, "score": 1, "kind": "private-value" }] });
        assert_eq!(
            first_mismatch(&s, &bad_enum).unwrap().to_string(),
            "/items/0/kind: expected enum, found string"
        );

        let nullable = json!({ "items": [{ "id": 1, "score": 1, "summary": 7 }] });
        assert_eq!(
            first_mismatch(&s, &nullable).unwrap().to_string(),
            "/items/0/summary: expected null or string, found integer"
        );
    }

    #[test]
    fn a_shape_path_maps_to_its_id_under_the_base() {
        let base = "https://schemas.inorbit.hr/eye/";
        assert_eq!(
            schema_id(base, "schemas/api/items/v0.json").as_deref(),
            Some("https://schemas.inorbit.hr/eye/api/items/v0.json")
        );
        assert_eq!(schema_id(base, "api/items/v0.json"), None);
        assert_eq!(schema_id(base, "schemas/../secret.json"), None);
        let v = Arc::new(schema());
        let by_closure = move |_: &str| Ok(v.clone());
        assert_eq!(
            check_body(&by_closure, "schemas/x/y/v0.json", br#"{"items":[]}"#),
            Ok(())
        );
    }

    struct One(Arc<Validator>);
    impl Shapes for One {
        fn validator(&self, _: &str) -> Result<Arc<Validator>, String> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn a_body_that_is_not_json_says_where_not_what() {
        let s = One(Arc::new(schema()));
        assert_eq!(check_body(&s, "x", br#"{"items":[]}"#), Ok(()));
        let e = check_body(&s, "x", b"<html>secret</html>").unwrap_err();
        assert!(
            e.starts_with("/: expected JSON, found a parse error at line 1"),
            "{e}"
        );
        assert!(!e.contains("secret"));
    }
}
