//! The cut hash: what the gateway stamps in `info.x-iohr-cut.hash`, computed here for a
//! document that carries no stamp (RFC 0020).
//!
//! The hash covers a projection of the document: `paths` and `components` with every
//! `description`, `summary`, `example`, `examples` and `x-iohr-*` key removed, at every
//! level, serialised as compact JSON with keys sorted at every level. Prose edits
//! never move it, and neither does a version bump in `info`; a route or a field does.
//! The platform computes it the same way; one test vector is shared with it.

use serde_json::{Map, Value};
use sha2::{Digest as _, Sha256};

const DROPPED: [&str; 4] = ["description", "summary", "example", "examples"];

/// The projection the hash covers, as a value.
#[must_use]
pub fn projection(doc: &Value) -> Value {
    let mut out = Map::new();
    out.insert(
        "paths".into(),
        strip(doc.get("paths").unwrap_or(&Value::Null)),
    );
    out.insert(
        "components".into(),
        strip(doc.get("components").unwrap_or(&Value::Null)),
    );
    Value::Object(out)
}

fn strip(node: &Value) -> Value {
    match node {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, _)| !DROPPED.contains(&k.as_str()) && !k.starts_with("x-iohr-"))
                .map(|(k, v)| (k.clone(), strip(v)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(strip).collect()),
        other => other.clone(),
    }
}

/// Compact JSON with keys sorted at every level: the canonical form the hash reads.
#[must_use]
pub fn canonical(value: &Value) -> Vec<u8> {
    let sorted = sort(value);
    serde_json::to_vec(&sorted).unwrap_or_default()
}

fn sort(value: &Value) -> Value {
    match value {
        Value::Object(m) => {
            // serde_json's map is ordered by key when `preserve_order` is off; sorting
            // explicitly keeps the hash the same whatever a dependency graph enables.
            let mut pairs: Vec<(&String, &Value)> = m.iter().collect();
            pairs.sort_by(|a, b| a.0.cmp(b.0));
            let mut out = Map::new();
            for (k, v) in pairs {
                out.insert(k.clone(), sort(v));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(sort).collect()),
        other => other.clone(),
    }
}

/// `sha256:<hex>` over the canonical projection of `doc`.
#[must_use]
pub fn cut_hash(doc: &Value) -> String {
    let digest = Sha256::digest(canonical(&projection(doc)));
    let mut hex = String::with_capacity(71);
    hex.push_str("sha256:");
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{canonical, cut_hash, projection};

    #[test]
    fn prose_and_info_do_not_move_the_hash_but_a_route_does() {
        let a = json!({
            "info": { "version": "1.0.0", "x-iohr-cut": { "hash": "x" } },
            "paths": { "/v1/me": { "get": { "operationId": "me", "summary": "s", "description": "d",
                "x-iohr-scopes": ["identity:read"], "responses": { "200": { "description": "ok" } } } } },
            "components": { "schemas": { "Me": { "type": "object", "description": "who", "example": {}, "properties": { "subject": { "type": "string" } } } } }
        });
        let mut b = a.clone();
        b["info"]["version"] = json!("2.0.0");
        b["paths"]["/v1/me"]["get"]["summary"] = json!("other");
        b["components"]["schemas"]["Me"]["description"] = json!("other");
        assert_eq!(cut_hash(&a), cut_hash(&b));
        let mut c = a.clone();
        c["paths"]["/v1/other"] = json!({ "get": { "operationId": "other" } });
        assert_ne!(cut_hash(&a), cut_hash(&c));
        let mut d = a.clone();
        d["components"]["schemas"]["Me"]["properties"]["email"] = json!({ "type": "string" });
        assert_ne!(cut_hash(&a), cut_hash(&d));
        assert!(cut_hash(&a).starts_with("sha256:") && cut_hash(&a).len() == 71);
    }

    #[test]
    fn the_canonical_form_sorts_keys_and_drops_nothing_else() {
        let v = json!({ "b": [ { "y": 1, "x": 2 } ], "a": null });
        assert_eq!(canonical(&v), br#"{"a":null,"b":[{"x":2,"y":1}]}"#);
        let p = projection(
            &json!({ "paths": { "/p": { "x-iohr-public": true, "get": { "description": "d" } } } }),
        );
        assert_eq!(
            p,
            json!({ "paths": { "/p": { "get": {} } }, "components": null })
        );
    }
}
