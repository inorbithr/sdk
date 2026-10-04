//! The rules of `spec/README.md`, as `tools/spec-sync.py` applies them, in Rust: short
//! schema names (N1), and the error envelope as its own schema. The facts the platform
//! states itself since core #218 (answers' `required` fields, `Detail`'s discriminator,
//! the production server, `Code`'s `x-http-status`; the former rules N2, N3, N4, N6)
//! are checked, never patched in: a document without them fails. The two
//! implementations are kept equal by a test that compares them byte for byte on the
//! same document.
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
/// The production server, which the public document names first (former N4).
pub const SERVER_URL: &str = "https://api.inorbit.hr";
/// A normalised document and the error envelope as a JSON Schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normalised {
    /// The OpenAPI document, rules applied.
    pub openapi: Value,
    /// `spec/problem.json`: the `Problem` envelope with `Code.x-http-status`, as the
    /// document states it.
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

fn reached_from(
    node: &Value,
    schemas: &Map<String, Value>,
) -> Result<BTreeSet<String>, NormaliseError> {
    let mut roots = BTreeSet::new();
    refs(node, &mut roots);
    reachable(schemas, roots)
}

/// The schemas only an answer reaches: no request body or parameter leads to them.
fn response_only(
    paths: &Map<String, Value>,
    schemas: &Map<String, Value>,
) -> Result<BTreeSet<String>, NormaliseError> {
    let mut asked = Map::new();
    let mut answered = Map::new();
    for (path, item) in paths {
        let Some(item) = item.as_object() else {
            continue;
        };
        for (method, op) in item.iter().filter(|(m, _)| is_method(m)) {
            let key = format!("{method} {path}");
            for part in ["requestBody", "parameters"] {
                if let Some(v) = op.get(part) {
                    asked.insert(format!("{key} {part}"), v.clone());
                }
            }
            if let Some(v) = op.get("responses") {
                answered.insert(key, v.clone());
            }
        }
        if let Some(v) = item.get("parameters") {
            asked.insert(format!("{path} parameters"), v.clone());
        }
    }
    let asked = reached_from(&Value::Object(asked), schemas)?;
    let answered = reached_from(&Value::Object(answered), schemas)?;
    Ok(answered.difference(&asked).cloned().collect())
}

fn regressed(rule: &str, detail: impl Into<String>) -> NormaliseError {
    NormaliseError::Regressed {
        rule: rule.to_owned(),
        detail: detail.into(),
    }
}

/// Former N2, fixed upstream (core #218): a message only an answer carries lists its
/// fields without presence as `required`; a request-side message marks nothing. A
/// document whose answers mark nothing, or name a field they do not define, fails.
fn check_required(
    paths: &Map<String, Value>,
    schemas: &Map<String, Value>,
) -> Result<(), NormaliseError> {
    let answers: Vec<String> = response_only(paths, schemas)?
        .into_iter()
        .filter(|n| {
            n.contains('.')
                && !n.starts_with("google.")
                && schemas[n].get("type") == Some(&json!("object"))
        })
        .collect();
    let props_of = |n: &str| {
        schemas[n]
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    };
    let required_of = |n: &str| -> Vec<String> {
        schemas[n]
            .get("required")
            .and_then(Value::as_array)
            .map(|r| {
                r.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    for name in &answers {
        let props = props_of(name);
        let mut unknown: Vec<String> = required_of(name)
            .into_iter()
            .filter(|r| !props.contains_key(r))
            .collect();
        unknown.sort();
        if !unknown.is_empty() {
            return Err(regressed(
                "N2",
                format!(
                    "{name} requires fields it does not define: {}",
                    format!("{unknown:?}").replace('"', "'")
                ),
            ));
        }
    }
    let with_scalars = answers
        .iter()
        .any(|n| props_of(n).values().any(|p| p.get("$ref").is_none()));
    if with_scalars && !answers.iter().any(|n| !required_of(n).is_empty()) {
        return Err(regressed(
            "N2",
            "no answer's message marks a field required; the platform states which fields are always sent since core #218, so the document regressed upstream",
        ));
    }
    Ok(())
}

/// Former N4, fixed upstream (core #218): the document names its server by an absolute
/// https URL ([`SERVER_URL`] in production), not `/`.
fn check_server(raw: &Value) -> Result<(), NormaliseError> {
    let url = raw.pointer("/servers/0/url").and_then(Value::as_str);
    if url.is_some_and(|u| u.len() > "https://".len() && u.starts_with("https://")) {
        return Ok(());
    }
    let shown = url.map_or_else(|| "None".to_owned(), |u| format!("'{u}'"));
    Err(regressed(
        "N4",
        format!(
            "servers[0].url is {shown}, not an absolute https URL such as {SERVER_URL}; the document regressed upstream (core #218)"
        ),
    ))
}

/// Former N3 and N6, fixed upstream (core #218): `Detail` names its discriminator and
/// `Code` carries `x-http-status`, a status for every code and no other.
fn check_envelope(schemas: &Map<String, Value>) -> Result<(), NormaliseError> {
    for name in ["Problem", "Code", "Detail"] {
        if !schemas.contains_key(name) {
            return Err(NormaliseError::Problem(format!(
                "the document has no {name} schema"
            )));
        }
    }
    if schemas["Detail"].pointer("/discriminator/propertyName") != Some(&json!("type")) {
        return Err(regressed(
            "N3",
            "Detail has no discriminator on `type`; the document regressed upstream (core #218)",
        ));
    }
    let codes: BTreeSet<&str> = schemas["Code"]
        .get("enum")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let Some(table) = schemas["Code"]
        .get("x-http-status")
        .and_then(Value::as_object)
    else {
        return Err(NormaliseError::Problem(
            "Code has no x-http-status; the document regressed upstream (core #218)".into(),
        ));
    };
    let list = |v: Vec<&str>| {
        if v.is_empty() {
            "none".to_owned()
        } else {
            format!("{v:?}").replace('"', "'")
        }
    };
    let missing: Vec<&str> = codes
        .iter()
        .copied()
        .filter(|c| !table.contains_key(*c))
        .collect();
    let extra: Vec<&str> = table
        .keys()
        .map(String::as_str)
        .filter(|c| !codes.contains(c))
        .collect();
    let bad: Vec<&str> = table
        .iter()
        .filter(|(_, s)| !s.as_u64().is_some_and(|s| (100..=599).contains(&s)))
        .map(|(c, _)| c.as_str())
        .collect();
    if !missing.is_empty() || !extra.is_empty() || !bad.is_empty() {
        return Err(NormaliseError::Problem(format!(
            "Code's x-http-status does not match its codes (codes without a status: {}; statuses for no code: {}; not an HTTP status: {})",
            list(missing),
            list(extra),
            list(bad)
        )));
    }
    Ok(())
}

/// `problem.json`: each `Detail` variant becomes its own definition, named
/// `<Type>Detail`, and `Detail` a `oneOf` over them with the document's discriminator.
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
    defs.insert("Detail".into(), Value::Object(detail));
    Ok(())
}

/// The error envelope as a JSON Schema: `Problem`, with `Code` (and its
/// `x-http-status`) and each `Detail` variant as its own definition.
fn problem_schema(schemas: &Map<String, Value>) -> Result<Value, NormaliseError> {
    let names: BTreeMap<String, String> = [("Code", "Code"), ("Detail", "Detail")]
        .into_iter()
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
        .collect();
    let mut defs = Map::new();
    defs.insert("Code".into(), schemas["Code"].clone());

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

/// Applies N1 to `raw` and checks the facts the platform states itself (the former N2,
/// N3, N4, N6), keeping every operation it holds.
///
/// # Errors
///
/// [`NormaliseError`] when the document is not OpenAPI 3.1, has no operation, points at
/// a schema it does not define, has a name clash N1 cannot settle, or lacks one of the
/// facts the platform states since core #218.
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
    let kept = reached_from(&Value::Object(paths.clone()), &all_schemas)?;
    check_server(raw)?;
    check_required(&paths, &all_schemas)?;
    let names = short_names(&kept)?;

    let mut schemas = Map::new();
    for original in &kept {
        schemas.insert(
            names[original].clone(),
            rewrite_refs(&all_schemas[original], &names, SCHEMAS),
        );
    }
    check_envelope(&schemas)?;

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
        raw.get("servers").cloned().unwrap_or(Value::Null),
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
            "servers": [ { "url": "https://api.inorbit.hr", "description": "The API" } ],
            "paths": {
                "/v1/me": { "get": { "operationId": "me", "x-iohr-public": true,
                    "responses": { "200": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Me" } } } },
                                   "default": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Problem" } } } } } } },
                "/v1/plan-only": { "post": { "operationId": "PlanService.Thing",
                    "requestBody": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/iohr.plan.v1.ThingRequest" } } } },
                    "responses": { "200": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/iohr.plan.v1.Thing" } } } },
                                   "default": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Problem" } } } } } } }
            },
            "components": { "schemas": {
                "Me": { "type": "object", "properties": { "subject": { "type": "string" } } },
                "iohr.plan.v1.ThingRequest": { "type": "object", "properties": { "a": { "type": "string" } } },
                "iohr.plan.v1.Thing": { "type": "object", "required": ["b", "a"], "properties": { "b": { "type": "string" }, "a": { "type": "string" } } },
                "Problem": { "type": "object", "properties": { "code": { "$ref": "#/components/schemas/Code" }, "details": { "type": "array", "items": { "$ref": "#/components/schemas/Detail" } } } },
                "Code": { "type": "string", "enum": ["not_found", "timeout"], "x-http-status": { "not_found": 404, "timeout": 504 } },
                "Detail": { "discriminator": { "propertyName": "type" }, "oneOf": [ { "type": "object", "required": ["type"], "properties": { "type": { "const": "retry" }, "after_seconds": { "type": "integer" } } } ] }
            } }
        })
    }

    #[test]
    fn normalise_keeps_plan_routes_and_public_only_drops_them() {
        let doc = minimal();
        let all = normalise(&doc).unwrap();
        let paths = all.openapi["paths"].as_object().unwrap();
        assert!(paths.contains_key("/v1/plan-only") && paths.contains_key("/v1/me"));
        // The document's own `required`, discriminator, status table and server pass
        // through untouched; nothing is added to a request.
        let schemas = &all.openapi["components"]["schemas"];
        assert_eq!(schemas["Thing"]["required"], json!(["b", "a"]));
        assert!(schemas["ThingRequest"].get("required").is_none());
        assert!(schemas["Me"].get("required").is_none());
        assert_eq!(all.openapi["servers"], doc["servers"]);
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
    fn a_fact_missing_upstream_fails_instead_of_being_patched_in() {
        let fails = |edit: &dyn Fn(&mut serde_json::Value), rule: &str| {
            let mut doc = minimal();
            edit(&mut doc);
            let err = normalise(&doc).unwrap_err().to_string();
            assert!(err.starts_with(&format!("{rule}:")), "{rule}: {err}");
        };
        fails(
            &|d| {
                d["components"]["schemas"]["iohr.plan.v1.Thing"]
                    .as_object_mut()
                    .unwrap()
                    .remove("required");
            },
            "N2",
        );
        fails(
            &|d| d["components"]["schemas"]["iohr.plan.v1.Thing"]["required"] = json!(["c"]),
            "N2",
        );
        fails(
            &|d| {
                d["components"]["schemas"]["Detail"]
                    .as_object_mut()
                    .unwrap()
                    .remove("discriminator");
            },
            "N3",
        );
        fails(&|d| d["servers"] = json!([{ "url": "/" }]), "N4");
        fails(
            &|d| {
                d["components"]["schemas"]["Code"]
                    .as_object_mut()
                    .unwrap()
                    .remove("x-http-status");
            },
            "N6",
        );
        fails(
            &|d| {
                d["components"]["schemas"]["Code"]["enum"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!("brand_new"));
            },
            "N6",
        );
        fails(
            &|d| d["components"]["schemas"]["Code"]["x-http-status"]["timeout"] = json!("504"),
            "N6",
        );
        let mut doc = minimal();
        doc["openapi"] = json!("3.0.3");
        assert!(matches!(
            normalise(&doc).unwrap_err(),
            NormaliseError::Version(_)
        ));
    }
}
