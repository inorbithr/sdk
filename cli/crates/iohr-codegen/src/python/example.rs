//! One operation's example in Python, on `inorbithr` (see [`crate::examples`]).

use std::fmt::Write as _;

use serde_json::Value;

use super::{PyNaming, field_name};
use crate::context::Naming as _;
use crate::examples::{Call, Flow, client, scalar, strings};
use crate::ir::Type;

/// The example of `call`.
pub(crate) fn example(call: &Call<'_>) -> String {
    let naming = PyNaming;
    let c = client::PYTHON;
    let mut names: Vec<String> = c.imports.iter().map(|s| (*s).to_owned()).collect();
    names.push("ApiError".into());
    if let Some((model, _)) = &call.body {
        names.push(model.clone());
    }
    names.sort_unstable();
    names.dedup();
    let mut out = format!("from inorbithr import {}\n\n", names.join(", "));
    for line in c.lines {
        let _ = writeln!(out, "{line}");
    }
    out.push_str("\ntry:\n");
    let receiver = match call.tag {
        Some(tag) => format!("{}.{}", client::VAR, naming.method_name(tag)),
        None => client::VAR.to_owned(),
    };
    let method = naming.method_name(&call.op.name);
    let args = arguments(call);
    match &call.flow {
        Flow::Unary => {
            let _ = write!(
                out,
                "    value = {receiver}.{method}({args}).value\n    print(value)\n"
            );
        }
        Flow::Pages(_) => {
            let item = call.item();
            let _ = write!(
                out,
                "    for {item} in {receiver}.all_{method}({args}):\n        print({item})\n"
            );
        }
        Flow::Stream => {
            let _ = write!(
                out,
                "    with {receiver}.{method}({args}) as stream:\n        for event in stream:\n            print(event)\n"
            );
        }
    }
    out.push_str(
        "except ApiError as e:\n    print(f\"{e.code}: {e.problem} (request id {e.raw.request_id})\")\n",
    );
    out
}

/// The arguments: the path values, the body, the required query parameters by keyword.
fn arguments(call: &Call<'_>) -> String {
    let naming = PyNaming;
    let mut args: Vec<String> = call
        .path
        .iter()
        .map(|v| scalar(&Value::from(v.as_str())))
        .collect();
    if let Some((model, fields)) = &call.body {
        let fields: Vec<String> = fields
            .iter()
            .map(|f| format!("{}={}", field_name(&f.name).0, literal(&f.ty, &f.value)))
            .collect();
        if fields.len() > 1 {
            let mut text = format!("{model}(\n");
            for f in &fields {
                let _ = writeln!(text, "        {f},");
            }
            text.push_str("    )");
            args.push(text);
        } else {
            args.push(format!("{model}({})", fields.join(", ")));
        }
    }
    for (name, ty, value) in &call.query {
        args.push(format!(
            "{}={}",
            naming.field_name(name),
            literal(ty, value)
        ));
    }
    args.join(", ")
}

fn literal(ty: &Type, value: &Value) -> String {
    match (ty, value) {
        (Type::Array { .. }, _) => format!("[{}]", strings(value).join(", ")),
        (_, Value::Bool(true)) => "True".into(),
        (_, Value::Bool(false)) => "False".into(),
        _ => scalar(value),
    }
}
