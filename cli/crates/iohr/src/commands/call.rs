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
