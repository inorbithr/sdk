//! The models, as types: typify turns the normalised schemas into Rust, and
//! prettyplease formats them. Three schemas are not generated but mapped to the
//! runtime's own types, which carry behaviour typify cannot express: `Code` keeps an
//! unknown slug, `Detail` keeps an unknown variant, and `Int64` reads a decimal string
//! or a number.

use serde_json::{Value, json};
use typify::{TypeSpace, TypeSpaceImpl, TypeSpaceSettings};

use crate::target::RenderError;

/// The synthetic schema every `{type: string, format: int64}` becomes, so typify maps
/// it to the runtime's `Int64` instead of a plain `String`.
const INT64: &str = "Int64";

/// Rewrites every `{type: string, format: int64}` into a `$ref` to [`INT64`], keeping
/// the description.
fn rewrite_int64(node: &mut Value) {
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

/// N2 marks every field of a transcoded message required, and the gateway does send
/// every scalar. A field that is itself a message (a `$ref` to an object) is left out
/// of the answer when it was never set, so on the wire it is optional: the Rust type
/// makes it an `Option` and reads a missing one as `None`.
fn relax_message_fields(schemas: &mut Value) {
    let Value::Object(all) = schemas else {
        return;
    };
    let is_object = |name: &str, all: &serde_json::Map<String, Value>| {
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

/// Renders the schemas as a Rust module body (no header). `runtime` is the path of the
/// runtime crate (`inorbithr`, or `crate` inside it).
///
/// # Errors
///
/// [`RenderError::Models`] when typify cannot read a schema.
pub(crate) fn render(
    schemas: &serde_json::Map<String, Value>,
    runtime: &str,
) -> Result<String, RenderError> {
    let mut definitions = Value::Object(schemas.clone());
    relax_message_fields(&mut definitions);
    rewrite_int64(&mut definitions);
    definitions[INT64] =
        json!({ "type": "string", "description": "A 64-bit integer as a decimal string." });
    let root = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "definitions": definitions,
    });
    let root: schemars::schema::RootSchema = serde_json::from_value(root)
        .map_err(|e| RenderError::Models(format!("the schemas do not read as JSON Schema: {e}")))?;

    let mut settings = TypeSpaceSettings::default();
    settings
        .with_derive("PartialEq".into())
        .with_map_type("::std::collections::BTreeMap")
        .with_replacement(
            INT64,
            format!("{runtime}::Int64"),
            [TypeSpaceImpl::Display].into_iter(),
        )
        .with_replacement(
            "Code",
            format!("{runtime}::Code"),
            [TypeSpaceImpl::Display].into_iter(),
        )
        .with_replacement("Detail", format!("{runtime}::Detail"), [].into_iter());
    let mut space = TypeSpace::new(&settings);
    space
        .add_root_schema(root)
        .map_err(|e| RenderError::Models(e.to_string()))?;
    let stream = space.to_stream();
    let file: syn::File = syn::parse2(stream).map_err(|e| RenderError::Syntax {
        file: "models.rs",
        reason: e.to_string(),
    })?;
    Ok(prettyplease::unparse(&file))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::render;

    #[test]
    fn int64_strings_become_the_runtime_type_and_output_is_stable() {
        let schemas = json!({
            "Digest": { "type": "object", "required": ["id", "title", "when"], "properties": {
                "id": { "type": "string", "format": "int64", "description": "The id." },
                "title": { "type": "string" },
                "when": { "type": ["string", "null"] },
                "tags": { "type": "object", "additionalProperties": { "type": "string" } }
            } },
            "Code": { "type": "string", "enum": ["a", "b"] },
            "Detail": { "oneOf": [ { "type": "object", "required": ["type"], "properties": { "type": { "const": "retry" } } } ] },
            "Problem": { "type": "object", "properties": { "code": { "$ref": "#/components/schemas/Code" }, "details": { "type": "array", "items": { "$ref": "#/components/schemas/Detail" } } } }
        });
        let a = render(schemas.as_object().unwrap(), "inorbithr").unwrap();
        let b = render(schemas.as_object().unwrap(), "inorbithr").unwrap();
        assert_eq!(a, b);
        assert!(a.contains("pub id: inorbithr::Int64"), "{a}");
        assert!(
            a.contains("pub when: ::std::option::Option<::std::string::String>"),
            "{a}"
        );
        assert!(a.contains("::std::collections::BTreeMap"), "{a}");
        assert!(
            a.contains("inorbithr::Code>") && a.contains("Vec<inorbithr::Detail>"),
            "{a}"
        );
        assert!(
            !a.contains("pub enum Code"),
            "the runtime's Code replaces the generated one"
        );
    }

    #[test]
    fn a_required_message_field_reads_as_optional() {
        let schemas = json!({
            "Change": { "type": "object", "required": ["title", "example"], "properties": {
                "title": { "type": "string" },
                "example": { "$ref": "#/components/schemas/Example" }
            } },
            "Example": { "type": "object", "required": ["code"], "properties": { "code": { "type": "string" } } }
        });
        let out = render(schemas.as_object().unwrap(), "inorbithr").unwrap();
        assert!(
            out.contains("pub example: ::std::option::Option<Example>"),
            "{out}"
        );
        assert!(out.contains("pub title: ::std::string::String"), "{out}");
    }
}
