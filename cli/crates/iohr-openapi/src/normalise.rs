//! The rules N1 to N6 of `spec/README.md`, as `tools/spec-sync.py` applies them, in
//! Rust: short schema names, every field of a transcoded message required,
//! discriminators on `type` or `kind`, the production server, and the error envelope
//! as its own schema with the code-to-status table. The two implementations are kept
//! equal by a test that compares them byte for byte on the same document.
//!
//! Unlike the sync tool, [`normalise`] keeps every operation a document holds: a
//! credential's document already is the cut, plan routes included. The sync tool's
//! public-only filter is [`public_only`], applied first when that is wanted.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::error::NormaliseError;

const SCHEMAS: &str = "#/components/schemas/";
const DEFS: &str = "#/$defs/";
/// The methods an OpenAPI path item may carry.
pub const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];
const DISCRIMINATORS: [&str; 2] = ["type", "kind"];

/// N6: the HTTP status of every error code, the platform's `Code::http` table.
pub const HTTP_STATUS: [(&str, u16); 17] = [
    ("bad_request", 400),
    ("failed_precondition", 400),
    ("unauthenticated", 401),
    ("forbidden", 403),
    ("not_found", 404),
    ("method_not_allowed", 405),
    ("already_exists", 409),
    ("conflict", 409),
    ("payload_too_large", 413),
    ("unsupported_media_type", 415),
    ("rate_limited", 429),
    ("quota_exceeded", 429),
    ("cancelled", 499),
    ("internal", 500),
    ("unimplemented", 501),
    ("unavailable", 503),
    ("timeout", 504),
];

/// A normalised document and the error envelope as a JSON Schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normalised {
    /// The OpenAPI document, rules applied.
    pub openapi: Value,
    /// `spec/problem.json`: the `Problem` envelope with `Code.x-http-status`.
    pub problem: Value,
}

/// Whether `key` is an HTTP method of a path item.
#[must_use]
pub fn is_method(key: &str) -> bool {
    METHODS.contains(&key)
}

/// The sync tool's filter: the paths with only their `x-iohr-public` operations.
///
/// # Errors
///
/// [`NormaliseError::NothingPublic`] when no operation is marked.
pub fn public_only(doc: &Value) -> Result<Value, NormaliseError> {
    let mut out = doc.clone();
    let mut kept = Map::new();
    if let Some(paths) = doc.get("paths").and_then(Value::as_object) {
        for (path, item) in paths {
            let Some(item) = item.as_object() else {
                continue;
            };
            let ops: Map<String, Value> = item
                .iter()
                .filter(|(m, op)| {
                    is_method(m) && op.get("x-iohr-public") == Some(&Value::Bool(true))
                })
                .map(|(m, op)| (m.clone(), op.clone()))
                .collect();
            if ops.is_empty() {
                continue;
            }
            let mut shared: Map<String, Value> = item
                .iter()
                .filter(|(m, _)| !is_method(m))
                .map(|(m, v)| (m.clone(), v.clone()))
                .collect();
            shared.extend(ops);
            kept.insert(path.clone(), Value::Object(shared));
        }
    }
    if kept.is_empty() {
        return Err(NormaliseError::NothingPublic);
    }
    out["paths"] = Value::Object(kept);
    Ok(out)
}

/// Every schema name a node points at through `$ref`.
fn refs(node: &Value, found: &mut BTreeSet<String>) {
    match node {
        Value::Object(m) => {
            if let Some(Value::String(r)) = m.get("$ref")
                && let Some(name) = r.strip_prefix(SCHEMAS)
            {
                found.insert(name.to_owned());
            }
            m.values().for_each(|v| refs(v, found));
        }
        Value::Array(a) => a.iter().for_each(|v| refs(v, found)),
        _ => {}
    }
}

fn rewrite_refs(node: &Value, names: &BTreeMap<String, String>, prefix: &str) -> Value {
    match node {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| {
                    if k == "$ref"
                        && let Some(name) = v.as_str().and_then(|s| s.strip_prefix(SCHEMAS))
                    {
                        let new = names.get(name).map_or(name, String::as_str);
                        (k.clone(), Value::String(format!("{prefix}{new}")))
                    } else {
                        (k.clone(), rewrite_refs(v, names, prefix))
                    }
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(|v| rewrite_refs(v, names, prefix)).collect()),
        other => other.clone(),
    }
}

fn reachable(
    schemas: &Map<String, Value>,
    roots: BTreeSet<String>,
) -> Result<BTreeSet<String>, NormaliseError> {
    let mut seen = BTreeSet::new();
    let mut todo: Vec<String> = roots.into_iter().collect();
    while let Some(name) = todo.pop() {
        if seen.contains(&name) {
            continue;
        }
        let Some(schema) = schemas.get(&name) else {
            return Err(NormaliseError::DanglingRef(name));
        };
        seen.insert(name.clone());
        let mut inner = BTreeSet::new();
        refs(schema, &mut inner);
        todo.extend(inner.into_iter().filter(|n| !seen.contains(n)));
    }
    Ok(seen)
}

/// N1: `iohr.accounts.v1.GetMeResponse` becomes `GetMeResponse`; a clash is told apart
/// by the proto package (`iohr.accounts.v1.Key` becomes `AccountsKey`).
fn short_names(names: &BTreeSet<String>) -> Result<BTreeMap<String, String>, NormaliseError> {
    let mut by_short: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in names {
        let short = name.rsplit('.').next().unwrap_or(name).to_owned();
        by_short.entry(short).or_default().push(name.clone());
    }
    let mut out = BTreeMap::new();
    for (short, group) in by_short {
        if group.len() == 1 {
            out.insert(group[0].clone(), short);
            continue;
        }
        for name in group {
            if name.contains('.') {
                let parts: Vec<&str> = name.split('.').collect();
                let package = if parts.len() >= 4 { parts[1] } else { parts[0] };
                let mut chars = package.chars();
                let capped = match chars.next() {
                    Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                };
                out.insert(name, format!("{capped}{short}"));
            } else {
                out.insert(name, short.clone());
            }
        }
    }
    let mut taken: BTreeMap<String, String> = BTreeMap::new();
    for (original, new) in &out {
        if let Some(first) = taken.get(new) {
            return Err(NormaliseError::NameClash {
                a: original.clone(),
                b: first.clone(),
                short: new.clone(),
            });
        }
        taken.insert(new.clone(), original.clone());
    }
    Ok(out)
}

/// N2: a transcoded message always carries every field, so every field is required.
fn require_all(schema: &mut Value) {
    if schema.get("type") == Some(&json!("object"))
        && let Some(props) = schema.get("properties").and_then(Value::as_object)
    {
        let required: Vec<Value> = props.keys().map(|k| Value::String(k.clone())).collect();
        schema["required"] = Value::Array(required);
    }
}

/// Whether every value of `schema` has `prop` set to one known string.
fn fixes(schema: &Value, prop: &str) -> bool {
    let spec = schema
        .pointer(&format!("/properties/{prop}"))
        .cloned()
        .unwrap_or(Value::Null);
    let single = spec.get("const").is_some()
        || spec
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|e| e.len() == 1);
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .is_some_and(|r| r.iter().any(|v| v == prop));
    single && required
}

/// N3: give a `oneOf` whose variants each fix `type` (or `kind`) a discriminator.
fn discriminate(node: &mut Value, schemas: &Map<String, Value>) {
    match node {
        Value::Object(m) => {
            if let Some(Value::Array(variants)) = m.get("oneOf")
                && !m.contains_key("discriminator")
            {
                let resolved: Vec<Value> = variants
                    .iter()
                    .map(|v| {
                        match v
                            .get("$ref")
                            .and_then(Value::as_str)
                            .and_then(|r| r.strip_prefix(SCHEMAS))
                        {
                            Some(name) => schemas.get(name).cloned().unwrap_or(Value::Null),
                            None => v.clone(),
                        }
                    })
                    .collect();
                for prop in DISCRIMINATORS {
                    if resolved.iter().all(|v| fixes(v, prop)) {
                        m.insert("discriminator".into(), json!({ "propertyName": prop }));
                        break;
                    }
                }
            }
            m.values_mut().for_each(|v| discriminate(v, schemas));
        }
        Value::Array(a) => a.iter_mut().for_each(|v| discriminate(v, schemas)),
        _ => {}
    }
}

fn status_table() -> BTreeMap<&'static str, u16> {
    HTTP_STATUS.into_iter().collect()
}

/// N6: each `Detail` variant becomes its own definition, named `<Type>Detail`, and
/// `Detail` a `oneOf` over them with a discriminator on `type`.
fn detail_defs(detail_schema: &Value, defs: &mut Map<String, Value>) -> Result<(), NormaliseError> {
    let mut variants = Vec::new();
    for variant in detail_schema
        .get("oneOf")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let kind = variant
            .pointer("/properties/type")
            .cloned()
            .unwrap_or(Value::Null);
        let value = kind.get("const").cloned().or_else(|| {
            kind.get("enum")
                .and_then(Value::as_array)
                .and_then(|e| e.first().cloned())
        });
        let Some(Value::String(value)) = value else {
            return Err(NormaliseError::Problem(
                "a Detail variant has no fixed `type`".into(),
            ));
        };
        let mut chars = value.chars();
        let name = match chars.next() {
            Some(c) => format!("{}{}Detail", c.to_uppercase(), chars.as_str()),
            None => "Detail".to_owned(),
        };
        defs.insert(name.clone(), variant.clone());
        variants.push(json!({ "$ref": format!("{DEFS}{name}") }));
    }
    let mut detail: Map<String, Value> = detail_schema
        .as_object()
        .map(|m| {
            m.iter()
                .filter(|(k, _)| *k != "oneOf")
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();
    detail.insert("oneOf".into(), Value::Array(variants));
    detail.insert("discriminator".into(), json!({ "propertyName": "type" }));
    defs.insert("Detail".into(), Value::Object(detail));
    Ok(())
}

/// N6: the error envelope as a JSON Schema, with the code-to-status table.
fn problem_schema(schemas: &Map<String, Value>) -> Result<Value, NormaliseError> {
    for name in ["Problem", "Code", "Detail"] {
        if !schemas.contains_key(name) {
            return Err(NormaliseError::Problem(format!(
                "the document has no {name} schema"
            )));
        }
    }
    let codes: Vec<String> = schemas["Code"]
        .get("enum")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let table = status_table();
    let missing: Vec<&str> = codes
        .iter()
        .map(String::as_str)
        .filter(|c| !table.contains_key(c))
        .collect();
    let gone: Vec<&str> = table
        .keys()
        .copied()
        .filter(|c| !codes.iter().any(|k| k == c))
        .collect();
    if !missing.is_empty() || !gone.is_empty() {
        let show = |v: &[&str]| {
            if v.is_empty() {
                "none".to_owned()
            } else {
                format!("{v:?}")
            }
        };
        return Err(NormaliseError::Problem(format!(
            "the status table no longer matches the codes (new codes without a status: {}; table codes the document dropped: {}); update HTTP_STATUS from the platform's Code::http",
            show(&missing),
            show(&gone)
        )));
    }

    let names: BTreeMap<String, String> = [("Code", "Code"), ("Detail", "Detail")]
        .into_iter()
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
        .collect();
    let mut defs = Map::new();
    let mut code = schemas["Code"].clone();
    let statuses: Map<String, Value> = codes
        .iter()
        .map(|c| (c.clone(), json!(table[c.as_str()])))
        .collect();
    code["x-http-status"] = Value::Object(statuses);
    defs.insert("Code".into(), code);

    detail_defs(&schemas["Detail"], &mut defs)?;

    let mut out = Map::new();
    out.insert(
        "$schema".into(),
        json!("https://json-schema.org/draft/2020-12/schema"),
    );
    out.insert(
        "$id".into(),
        json!("https://github.com/inorbithr/sdk/spec/problem.json"),
    );
    out.insert("title".into(), json!("Problem"));
    if let Value::Object(problem) = rewrite_refs(&schemas["Problem"], &names, DEFS) {
        out.extend(problem);
    }
    out.insert(
        "$defs".into(),
        rewrite_refs(&Value::Object(defs), &names, DEFS),
    );
    Ok(Value::Object(out))
}

/// Applies N1 to N6 to `raw`, keeping every operation it holds.
///
/// # Errors
///
/// [`NormaliseError`] when the document is not OpenAPI 3.1, has no operation, points at
/// a schema it does not define, has a name clash N1 cannot settle, or an error envelope
/// that does not match the status table.
pub fn normalise(raw: &Value) -> Result<Normalised, NormaliseError> {
    let version = raw
        .get("openapi")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !version.starts_with("3.1") {
        return Err(NormaliseError::Version(version.to_owned()));
    }
    let paths = raw
        .get("paths")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if !paths
        .values()
        .filter_map(Value::as_object)
        .any(|item| item.keys().any(|k| is_method(k)))
    {
        return Err(NormaliseError::NoOperation);
    }
    let all_schemas = raw
        .pointer("/components/schemas")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut roots = BTreeSet::new();
    refs(&Value::Object(paths.clone()), &mut roots);
    let kept = reachable(&all_schemas, roots)?;
    let names = short_names(&kept)?;

    let mut schemas = Map::new();
    for original in &kept {
        let mut schema = all_schemas[original].clone();
        if original.contains('.') {
            require_all(&mut schema);
        }
        schemas.insert(
            names[original].clone(),
            rewrite_refs(&schema, &names, SCHEMAS),
        );
    }
    let snapshot = schemas.clone();
    let mut schemas_value = Value::Object(schemas);
    discriminate(&mut schemas_value, &snapshot);
    let Value::Object(schemas) = schemas_value else {
        unreachable!("an object stays an object");
    };

    let mut components = Map::new();
    components.insert("schemas".into(), Value::Object(schemas.clone()));
    if let Some(s) = raw.pointer("/components/securitySchemes") {
        components.insert("securitySchemes".into(), s.clone());
    }
    let mut out = Map::new();
    out.insert("openapi".into(), raw["openapi"].clone());
    out.insert(
        "info".into(),
        raw.get("info").cloned().unwrap_or(Value::Null),
    );
    out.insert(
        "servers".into(),
        json!([{ "url": "https://api.inorbit.hr", "description": "The InOrbit API" }]),
    );
    out.insert(
        "paths".into(),
        rewrite_refs(&Value::Object(paths), &names, SCHEMAS),
    );
    out.insert("components".into(), Value::Object(components));
    if let Some(s) = raw.get("security") {
        out.insert("security".into(), s.clone());
    }
    let problem = problem_schema(&schemas)?;
    Ok(Normalised {
        openapi: Value::Object(out),
        problem,
    })
}

/// The file form the sync tool writes: pretty JSON, keys sorted, two spaces, a trailing
/// newline.
#[must_use]
pub fn render_json(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".into());
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::{NormaliseError, normalise, public_only, short_names};

    fn names(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn n1_shortens_and_tells_a_clash_apart_by_package() {
        let out = short_names(&names(&[
            "iohr.accounts.v1.GetMeResponse",
            "Key",
            "iohr.accounts.v1.Key",
        ]))
        .unwrap();
        assert_eq!(out["iohr.accounts.v1.GetMeResponse"], "GetMeResponse");
        assert_eq!(out["Key"], "Key");
        assert_eq!(out["iohr.accounts.v1.Key"], "AccountsKey");
        let err =
            short_names(&names(&["iohr.accounts.v1.Key", "iohr.accounts.v2.Key"])).unwrap_err();
        assert!(matches!(err, NormaliseError::NameClash { .. }), "{err}");
    }

    fn minimal() -> serde_json::Value {
        json!({
            "openapi": "3.1.0",
            "info": { "title": "t", "version": "1" },
            "paths": {
                "/v1/me": { "get": { "operationId": "me", "x-iohr-public": true,
                    "responses": { "200": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Me" } } } },
                                   "default": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Problem" } } } } } } },
                "/v1/plan-only": { "get": { "operationId": "PlanService.Thing",
                    "responses": { "200": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/iohr.plan.v1.Thing" } } } },
                                   "default": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Problem" } } } } } } }
            },
            "components": { "schemas": {
                "Me": { "type": "object", "properties": { "subject": { "type": "string" } } },
                "iohr.plan.v1.Thing": { "type": "object", "properties": { "b": { "type": "string" }, "a": { "type": "string" } } },
                "Problem": { "type": "object", "properties": { "code": { "$ref": "#/components/schemas/Code" }, "details": { "type": "array", "items": { "$ref": "#/components/schemas/Detail" } } } },
                "Code": { "type": "string", "enum": ["bad_request","failed_precondition","unauthenticated","forbidden","not_found","method_not_allowed","already_exists","conflict","payload_too_large","unsupported_media_type","rate_limited","quota_exceeded","cancelled","internal","unimplemented","unavailable","timeout"] },
                "Detail": { "oneOf": [ { "type": "object", "required": ["type"], "properties": { "type": { "const": "retry" }, "after_seconds": { "type": "integer" } } } ] }
            } }
        })
    }

    #[test]
    fn normalise_keeps_plan_routes_and_public_only_drops_them() {
        let doc = minimal();
        let all = normalise(&doc).unwrap();
        let paths = all.openapi["paths"].as_object().unwrap();
        assert!(paths.contains_key("/v1/plan-only") && paths.contains_key("/v1/me"));
        // N2 on the proto schema only, N3 on the envelope, N6 with the table.
        assert_eq!(
            all.openapi["components"]["schemas"]["Thing"]["required"],
            json!(["a", "b"])
        );
        assert!(
            all.openapi["components"]["schemas"]["Me"]
                .get("required")
                .is_none()
        );
        assert_eq!(
            all.problem["$defs"]["Detail"]["discriminator"]["propertyName"],
            "type"
        );
        assert_eq!(
            all.problem["$defs"]["Code"]["x-http-status"]["timeout"],
            504
        );
        let public = normalise(&public_only(&doc).unwrap()).unwrap();
        assert!(
            !public.openapi["paths"]
                .as_object()
                .unwrap()
                .contains_key("/v1/plan-only")
        );
        assert!(
            public.openapi["components"]["schemas"]
                .get("Thing")
                .is_none(),
            "unreached schemas go"
        );
        // Idempotent: normalising the output again changes nothing.
        assert_eq!(normalise(&all.openapi).unwrap(), all);
    }

    #[test]
    fn n6_refuses_a_code_the_table_does_not_know() {
        let mut doc = minimal();
        doc["components"]["schemas"]["Code"]["enum"]
            .as_array_mut()
            .unwrap()
            .push(json!("brand_new"));
        let err = normalise(&doc).unwrap_err();
        assert!(err.to_string().contains("brand_new"), "{err}");
        let mut doc = minimal();
        doc["openapi"] = json!("3.0.3");
        assert!(matches!(
            normalise(&doc).unwrap_err(),
            NormaliseError::Version(_)
        ));
    }
}
