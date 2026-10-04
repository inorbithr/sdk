use std::io::Read as _;

use inorbithr::Method;
use serde_json::{Map, Value};

use crate::Env;
use crate::cli::{ApiCall, Global, HttpMethod};
use crate::context::{response, session};
use crate::error::Error;
use crate::output::Out;
use inorbithr::error::MAX_BODY;

pub(crate) async fn run(g: &Global, env: &Env, call: ApiCall, out: Out) -> Result<(), Error> {
    let method = match call.method {
        HttpMethod::Get => Method::Get,
        HttpMethod::Post => Method::Post,
        HttpMethod::Put => Method::Put,
        HttpMethod::Patch => Method::Patch,
        HttpMethod::Delete => Method::Delete,
    };
    let mut fields: Vec<(String, Value)> = Vec::new();
    for f in &call.fields {
        let (k, v) = split(f)?;
        fields.push((k.to_owned(), Value::String(v.to_owned())));
    }
    for f in &call.typed_fields {
        let (k, v) = split(f)?;
        let parsed = serde_json::from_str(v).unwrap_or_else(|_| Value::String(v.to_owned()));
        fields.push((k.to_owned(), parsed));
    }
    let in_query = matches!(method, Method::Get | Method::Delete);
    let body = match &call.input {
        Some(path) => Some(read_body(path)?),
        None if !in_query && !fields.is_empty() => {
            Some(Value::Object(fields.iter().cloned().collect::<Map<_, _>>()))
        }
        None => None,
    };
    let query_text: Vec<(String, String)> = if in_query {
        fields
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.as_str().map_or_else(|| v.to_string(), str::to_owned),
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    let query: Vec<(&str, &str)> = query_text
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    if call.all {
        if !matches!(method, Method::Get) {
            return Err(Error::Usage(
                "--all pages a list, so it takes GET only".into(),
            ));
        }
        if query.iter().any(|(k, _)| *k == "page_token") {
            return Err(Error::Usage(
                "--all sets page_token itself; leave it out, or drop --all to ask for one page"
                    .into(),
            ));
        }
        let s = session(g, env).await?;
        return walk(&s.api, &call, &query).await;
    }

    let s = session(g, env).await?;
    let resp = s
        .api
        .send(method, &call.path, &query, body.as_ref())
        .await?;
    let resp = response(resp);
    if call.include {
        Out::raw(
            format!(
                "HTTP {}\nrequest id: {}\n\n",
                resp.status,
                resp.request_id.as_deref().unwrap_or("-")
            )
            .as_bytes(),
        );
    }
    let is_json = resp
        .content_type
        .as_deref()
        .is_some_and(|c| c.contains("json"));
    match serde_json::from_slice::<Value>(&resp.body) {
        Ok(v) if is_json || out.json => Out::print_json(&v),
        _ => Out::raw(&resp.body),
    }
    Ok(())
}

/// Every page of the list at `call.path`, printed as one answer: the first page's
/// fields, with the list holding every page's items and `next_page_token` empty, or
/// still set when `--max-pages` stopped the walk (RFC 0033).
async fn walk(
    api: &crate::context::Api,
    call: &ApiCall,
    query: &[(&str, &str)],
) -> Result<(), Error> {
    let mut merged: Option<(Map<String, Value>, String)> = None;
    let mut token = String::new();
    for page in 1..=call.max_pages {
        let mut q = query.to_vec();
        if !token.is_empty() {
            q.push(("page_token", token.as_str()));
        }
        let resp = response(api.send(Method::Get, &call.path, &q, None).await?);
        if call.include {
            Out::raw(
                format!(
                    "HTTP {} (page {page})\nrequest id: {}\n",
                    resp.status,
                    resp.request_id.as_deref().unwrap_or("-")
                )
                .as_bytes(),
            );
        }
        let Ok(Value::Object(mut answer)) = serde_json::from_slice::<Value>(&resp.body) else {
            return Err(Error::Usage(format!(
                "{} did not answer a JSON object, so it is not a list --all can page",
                call.path
            )));
        };
        let next = match answer.get("next_page_token") {
            Some(Value::String(t)) => t.clone(),
            Some(Value::Null) | None if page == 1 => {
                // Not a paged list: the one answer is the whole answer.
                Out::note(&format!(
                    "{} does not page; printed its one answer",
                    call.path
                ));
                if call.include {
                    Out::raw(b"\n");
                }
                Out::print_json(&Value::Object(answer));
                return Ok(());
            }
            _ => String::new(),
        };
        match &mut merged {
            None => {
                let Some(field) = list_field(&resp.body) else {
                    return Err(Error::Usage(format!(
                        "{} answers a next_page_token but no list next to it",
                        call.path
                    )));
                };
                answer.remove("next_page_token");
                merged = Some((answer, field));
            }
            Some((first, field)) => {
                let items = match answer.remove(field.as_str()) {
                    Some(Value::Array(items)) => items,
                    _ => Vec::new(),
                };
                if let Some(Value::Array(all)) = first.get_mut(field.as_str()) {
                    all.extend(items);
                }
            }
        }
        if next.is_empty() || next == token {
            token = String::new();
            break;
        }
        token = next;
    }
    let Some((mut first, _)) = merged else {
        return Ok(());
    };
    if !token.is_empty() {
        Out::note(&format!(
            "stopped after {} pages; next_page_token in the answer goes on from here",
            call.max_pages
        ));
    }
    first.insert("next_page_token".into(), Value::String(token));
    if call.include {
        Out::raw(b"\n");
    }
    Out::print_json(&Value::Object(first));
    Ok(())
}

/// The list a paged answer carries: its first array field in the order the server
/// wrote them (RFC 0033 puts the items first; an older route has other fields before
/// them, never another list). Read off the wire, because `serde_json::Map` sorts its
/// keys and `party_ids` would then come before `transactions`.
fn list_field(body: &[u8]) -> Option<String> {
    use serde::de::{Deserializer as _, MapAccess, Visitor};

    struct FirstArray;
    impl<'de> Visitor<'de> for FirstArray {
        type Value = Option<String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a JSON object")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut found = None;
            while let Some(key) = map.next_key::<String>()? {
                let value: Value = map.next_value()?;
                if found.is_none() && value.is_array() {
                    found = Some(key);
                }
            }
            Ok(found)
        }
    }
    serde_json::Deserializer::from_slice(body)
        .deserialize_map(FirstArray)
        .ok()
        .flatten()
}

fn split(field: &str) -> Result<(&str, &str), Error> {
    field
        .split_once('=')
        .filter(|(k, _)| !k.is_empty())
        .ok_or_else(|| Error::Usage("a field is KEY=VALUE, such as -f limit=5".into()))
}

fn read_body(path: &std::path::Path) -> Result<Value, Error> {
    let mut text = String::new();
    let limit = MAX_BODY as u64;
    let read = if path.as_os_str() == "-" {
        std::io::stdin()
            .lock()
            .take(limit)
            .read_to_string(&mut text)
    } else {
        std::fs::File::open(path).and_then(|f| f.take(limit).read_to_string(&mut text))
    };
    read.map_err(|e| Error::Usage(format!("cannot read the body from {}: {e}", path.display())))?;
    serde_json::from_str(&text).map_err(|e| Error::Usage(format!("the body is not JSON: {e}")))
}
