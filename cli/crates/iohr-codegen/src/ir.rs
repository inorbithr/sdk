//! The models as a language-neutral type model, read once from the normalised
//! schemas. Every target but Rust (which hands the schemas to typify) renders its
//! types from this: an interface, a record, a dataclass or a struct per [`Model`],
//! and a type expression per [`Type`].
//!
//! Two rules apply before anything is read, the same for every language (design.md
//! section 12): a field that is itself a message is optional on the wire however the
//! schema lists it ([`relax_message_fields`]), and a decimal-string 64-bit integer is
//! [`Type::Int64`], which each runtime carries as its own type. `Code` and `Detail`
//! are the runtime's own types too ([`Model::runtime`]): they keep an unknown slug or
//! variant, which a generated type cannot.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Value, json};

/// The schemas the runtime provides; a target refers to them and never renders them.
pub const RUNTIME_TYPES: [&str; 2] = ["Code", "Detail"];

/// A type expression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Type {
    /// Another model, by its short name.
    Ref {
        /// The model's name.
        name: String,
    },
    /// A string.
    String,
    /// A 64-bit integer carried as a decimal string on the wire.
    Int64,
    /// An integer that travels as a JSON number.
    Integer {
        /// 32 for `format: int32`, 64 otherwise.
        bits: u8,
    },
    /// A floating-point number.
    Number,
    /// `true` or `false`.
    Bool,
    /// A list.
    Array {
        /// The element type.
        item: Box<Type>,
    },
    /// An object with string keys and values of one type.
    Map {
        /// The value type.
        value: Box<Type>,
    },
    /// One of a fixed set of strings, declared inline.
    Enum {
        /// The values, in the schema's order.
        values: Vec<String>,
    },
    /// Anything: a schema with no type the model can name.
    Any,
}

/// One field of an object model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Field {
    /// The wire name (`org_id`).
    pub name: String,
    /// The type.
    pub ty: Type,
    /// Whether the answer always carries it.
    pub required: bool,
    /// Whether it may be `null` on the wire.
    pub nullable: bool,
    /// Its description, if any.
    pub doc: String,
}

/// What a model is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Shape {
    /// An object with named fields, in name order.
    Object {
        /// The fields.
        fields: Vec<Field>,
    },
    /// A string with a fixed set of values.
    Enum {
        /// The values, in the schema's order.
        values: Vec<String>,
    },
    /// One of several models, told apart by a string property when the schema names one.
    Union {
        /// The variants.
        variants: Vec<Type>,
        /// The property whose value says which variant it is.
        discriminator: Option<String>,
    },
    /// Another name for a type.
    Alias {
        /// The type.
        ty: Type,
    },
}

/// One named model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Model {
    /// The short name (`Digest`).
    pub name: String,
    /// Its description, if any.
    pub doc: String,
    /// What it is.
    pub shape: Shape,
    /// `true` for the runtime's own types ([`RUNTIME_TYPES`]): refer to them, do not
    /// render them.
    pub runtime: bool,
}

/// Every model of `schemas`, sorted by name, with the two wire rules applied.
#[must_use]
pub fn models(schemas: &BTreeMap<String, Value>) -> Vec<Model> {
    let mut all: Map<String, Value> = schemas
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let mut wrapped = Value::Object(std::mem::take(&mut all));
    relax_message_fields(&mut wrapped);
    let Value::Object(all) = wrapped else {
        return Vec::new();
    };
    let mut out: Vec<Model> = all
        .iter()
        .map(|(name, schema)| Model {
            name: name.clone(),
            doc: doc_of(schema),
            shape: shape_of(schema),
            runtime: RUNTIME_TYPES.contains(&name.as_str()),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The type of a schema used in place (a field, a parameter, an array item).
#[must_use]
pub fn type_of(schema: &Value) -> Type {
    nullable_type(schema).0
}

/// The type of a schema and whether it admits `null`.
#[must_use]
pub fn nullable_type(schema: &Value) -> (Type, bool) {
    if let Some(name) = ref_name(schema) {
        return (Type::Ref { name }, false);
    }
    // `oneOf: [{type: null}, X]` and `anyOf` the same: X, nullable.
    for key in ["oneOf", "anyOf"] {
        if let Some(variants) = schema.get(key).and_then(Value::as_array) {
            let non_null: Vec<&Value> = variants.iter().filter(|v| !is_null(v)).collect();
            if non_null.len() == 1 && non_null.len() < variants.len() {
                return (type_of(non_null[0]), true);
            }
        }
    }
    let (ty, nullable) = match schema.get("type") {
        Some(Value::String(t)) => (t.as_str(), false),
        Some(Value::Array(ts)) => {
            let names: Vec<&str> = ts.iter().filter_map(Value::as_str).collect();
            let nullable = names.contains(&"null");
            let rest: Vec<&str> = names.into_iter().filter(|t| *t != "null").collect();
            match rest.as_slice() {
                [one] => (*one, nullable),
                _ => return (Type::Any, nullable),
            }
        }
        _ => return (Type::Any, false),
    };
    let format = schema.get("format").and_then(Value::as_str).unwrap_or("");
    let out = match ty {
        "string" if format == "int64" || format == "uint64" => Type::Int64,
        "string" => match schema.get("enum").and_then(Value::as_array) {
            Some(values) => Type::Enum {
                values: values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
            },
            None => Type::String,
        },
        // An int64 that the document sends as a number stays a JSON number of the
        // widest kind the language has; only the decimal-string form is Int64.
        "integer" => Type::Integer {
            bits: if format == "int32" { 32 } else { 64 },
        },
        "number" => Type::Number,
        "boolean" => Type::Bool,
        "array" => Type::Array {
            item: Box::new(schema.get("items").map_or(Type::Any, type_of)),
        },
        "object" => match schema.get("additionalProperties") {
            Some(v) if v.is_object() && schema.get("properties").is_none() => Type::Map {
                value: Box::new(type_of(v)),
            },
            _ => Type::Any,
        },
        _ => Type::Any,
    };
    (out, nullable)
}

fn shape_of(schema: &Value) -> Shape {
    if let Some(variants) = schema.get("oneOf").and_then(Value::as_array) {
        let non_null: Vec<&Value> = variants.iter().filter(|v| !is_null(v)).collect();
        if non_null.len() > 1 {
            return Shape::Union {
                variants: non_null.into_iter().map(type_of).collect(),
                discriminator: schema
                    .pointer("/discriminator/propertyName")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            };
        }
    }
    if schema.get("type") == Some(&json!("string"))
        && let Some(values) = schema.get("enum").and_then(Value::as_array)
    {
        return Shape::Enum {
            values: values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        };
    }
    if let Some(props) = schema.get("properties").and_then(Value::as_object) {
        let required: Vec<&str> = schema
            .get("required")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let mut fields: Vec<Field> = props
            .iter()
            .map(|(name, prop)| {
                let (ty, nullable) = nullable_type(prop);
                Field {
                    name: name.clone(),
                    ty,
                    required: required.contains(&name.as_str()),
                    nullable,
                    doc: doc_of(prop),
                }
            })
            .collect();
        fields.sort_by(|a, b| a.name.cmp(&b.name));
        return Shape::Object { fields };
    }
    if schema.get("type") == Some(&json!("object")) && schema.get("additionalProperties").is_none()
    {
        return Shape::Object { fields: Vec::new() };
    }
    Shape::Alias {
        ty: type_of(schema),
    }
}

fn ref_name(schema: &Value) -> Option<String> {
    schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.rsplit('/').next())
        .map(str::to_owned)
}

fn is_null(schema: &Value) -> bool {
    schema.get("type") == Some(&json!("null"))
}

fn doc_of(schema: &Value) -> String {
    schema
        .get("description")
        .and_then(Value::as_str)
        .map(|d| d.split("\n\n").next().unwrap_or(d).trim().to_owned())
        .unwrap_or_default()
}

/// N2 marks every field of a transcoded message required, and the gateway does send
/// every scalar. A field that is itself a message (a `$ref` to an object) is left out
/// of the answer when it was never set, so on the wire it is optional: every target
/// reads a missing one as no value.
pub fn relax_message_fields(schemas: &mut Value) {
    let Value::Object(all) = schemas else {
        return;
    };
    let is_object = |name: &str, all: &Map<String, Value>| {
        all.get(name).is_some_and(|s| {
            s.get("type") == Some(&json!("object")) || s.get("properties").is_some()
        })
    };
    let names: Vec<String> = all.keys().cloned().collect();
    for name in names {
        let refs: Vec<String> = all[&name]
            .get("properties")
            .and_then(Value::as_object)
            .map(|props| {
                props
                    .iter()
                    .filter_map(|(k, v)| {
                        let target = v.get("$ref")?.as_str()?.rsplit('/').next()?.to_owned();
                        is_object(&target, all).then_some(k.clone())
                    })
                    .collect()
            })
            .unwrap_or_default();
        if refs.is_empty() {
            continue;
        }
        if let Some(Value::Array(required)) = all.get_mut(&name).and_then(|s| s.get_mut("required"))
        {
            required.retain(|r| r.as_str().is_none_or(|r| !refs.iter().any(|k| k == r)));
        }
    }
}

/// The synthetic schema every `{type: string, format: int64}` becomes for typify, so
/// the Rust target maps it to the runtime's `Int64` instead of a plain `String`.
pub(crate) const INT64: &str = "Int64";

/// Rewrites every `{type: string, format: int64}` into a `$ref` to [`INT64`], keeping
/// the description. Only the Rust target needs this: the IR reads the format directly.
pub(crate) fn rewrite_int64(node: &mut Value) {
    match node {
        Value::Object(m) => {
            if m.get("type") == Some(&json!("string")) && m.get("format") == Some(&json!("int64")) {
                let description = m.get("description").cloned();
                m.clear();
                m.insert(
                    "$ref".into(),
                    json!(format!("#/components/schemas/{INT64}")),
                );
                if let Some(d) = description {
                    m.insert("description".into(), d);
                }
                return;
            }
            m.values_mut().for_each(rewrite_int64);
        }
        Value::Array(a) => a.iter_mut().for_each(rewrite_int64),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{Field, Shape, Type, models, type_of};

    fn schemas(v: &serde_json::Value) -> BTreeMap<String, serde_json::Value> {
        v.as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    #[test]
    fn fields_carry_types_nullability_and_the_message_rule() {
        let m = models(&schemas(&json!({
            "Digest": { "type": "object", "required": ["id", "title", "example"], "properties": {
                "id": { "type": "string", "format": "int64", "description": "The id.\n\nMore." },
                "title": { "type": "string" },
                "when": { "type": ["string", "null"] },
                "tags": { "type": "object", "additionalProperties": { "type": "string" } },
                "example": { "$ref": "#/components/schemas/Example" },
                "key": { "oneOf": [ { "type": "null" }, { "$ref": "#/components/schemas/Example" } ] }
            } },
            "Example": { "type": "object", "properties": { "code": { "type": "string" } } },
            "Code": { "type": "string", "enum": ["a", "b"] }
        })));
        let names: Vec<&str> = m.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Code", "Digest", "Example"]);
        assert!(m[0].runtime && !m[1].runtime);
        let Shape::Object { fields } = &m[1].shape else {
            panic!("Digest is an object")
        };
        let f = |n: &str| fields.iter().find(|f| f.name == n).unwrap().clone();
        assert_eq!(
            f("id"),
            Field {
                name: "id".into(),
                ty: Type::Int64,
                required: true,
                nullable: false,
                doc: "The id.".into()
            }
        );
        assert!(f("when").nullable && !f("when").required);
        assert_eq!(
            f("tags").ty,
            Type::Map {
                value: Box::new(Type::String)
            }
        );
        assert!(
            !f("example").required,
            "a message field is optional on the wire"
        );
        assert!(f("key").nullable);
        assert_eq!(
            f("key").ty,
            Type::Ref {
                name: "Example".into()
            }
        );
        assert_eq!(
            m[0].shape,
            Shape::Enum {
                values: vec!["a".into(), "b".into()]
            }
        );
    }

    #[test]
    fn unions_keep_their_discriminator() {
        let m = models(&schemas(&json!({
            "Detail": { "oneOf": [ { "$ref": "#/components/schemas/A" }, { "$ref": "#/components/schemas/B" } ],
                        "discriminator": { "propertyName": "type" } }
        })));
        assert_eq!(
            m[0].shape,
            Shape::Union {
                variants: vec![
                    Type::Ref { name: "A".into() },
                    Type::Ref { name: "B".into() }
                ],
                discriminator: Some("type".into()),
            }
        );
        assert_eq!(
            type_of(&json!({ "type": "integer", "format": "int32" })),
            Type::Integer { bits: 32 }
        );
        assert_eq!(
            type_of(&json!({ "type": "array", "items": { "type": "boolean" } })),
            Type::Array {
                item: Box::new(Type::Bool)
            }
        );
    }
}
