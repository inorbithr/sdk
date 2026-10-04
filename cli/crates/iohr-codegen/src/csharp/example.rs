//! One operation's example in C#, on `InOrbit.Sdk` (see [`crate::examples`]): top-level
//! statements, as a console program writes them.

use std::fmt::Write as _;

use heck::{ToLowerCamelCase as _, ToUpperCamelCase as _};
use serde_json::Value;

use super::{CsNaming, property};
use crate::context::Naming as _;
use crate::examples::{Call, Flow, client, scalar, strings};
use crate::ir::Type;

/// The example of `call`.
pub(crate) fn example(call: &Call<'_>) -> String {
    let naming = CsNaming;
    let c = client::CSHARP;
    let mut usings: Vec<String> = c.imports.iter().map(|s| (*s).to_owned()).collect();
    usings.push("System".into());
    usings.push("InOrbit.Sdk.Api".into());
    usings.sort_by(|a, b| {
        (!a.starts_with("System"), a.as_str()).cmp(&(!b.starts_with("System"), b.as_str()))
    });
    usings.dedup();
    let mut out = String::new();
    for u in &usings {
        let _ = writeln!(out, "using {u};");
    }
    out.push('\n');
    for line in c.lines {
        let _ = writeln!(out, "{line}");
    }
    out.push_str("try\n{\n");
    let receiver = match call.tag {
        Some(tag) => format!("{}.{}()", client::VAR, naming.method_name(tag)),
        None => client::VAR.to_owned(),
    };
    let method = call.op.name.to_upper_camel_case();
    let args = arguments(call);
    match &call.flow {
        Flow::Unary => {
            let _ = write!(
                out,
                "    var value = (await {receiver}.{method}Async({args})).Value;\n    Console.WriteLine(value);\n"
            );
        }
        Flow::Pages(_) => {
            let item = naming.escape(call.item().to_lower_camel_case());
            let _ = write!(
                out,
                "    await foreach (var {item} in {receiver}.All{method}Async({args}))\n    {{\n        Console.WriteLine({item});\n    }}\n"
            );
        }
        Flow::Stream => {
            let _ = write!(
                out,
                "    await foreach (var @event in {receiver}.{method}Async({args}))\n    {{\n        Console.WriteLine(@event);\n    }}\n"
            );
        }
    }
    out.push_str(
        "}\ncatch (ApiException e)\n{\n    Console.Error.WriteLine($\"{e.Code}: {e.Problem} (request id {e.Raw.RequestId})\");\n}\n",
    );
    out
}

/// The path values, the query record (`null` when none is required and a body follows),
/// the body.
fn arguments(call: &Call<'_>) -> String {
    let mut args: Vec<String> = call
        .path
        .iter()
        .map(|v| scalar(&Value::from(v.as_str())))
        .collect();
    if let Some(params) = &call.op.params_type {
        if !call.query.is_empty() {
            let inits: Vec<String> = call
                .query
                .iter()
                .map(|(name, ty, value)| {
                    format!("{} = {}", property(name, params), literal(ty, value))
                })
                .collect();
            args.push(object(params, &inits));
        } else if call.body.is_some() {
            args.push("null".into());
        }
    }
    if let Some((model, fields)) = &call.body {
        let inits: Vec<String> = fields
            .iter()
            .map(|f| {
                format!(
                    "{} = {}",
                    property(&f.name, model),
                    literal(&f.ty, &f.value)
                )
            })
            .collect();
        args.push(object(model, &inits));
    }
    args.join(", ")
}

fn literal(ty: &Type, value: &Value) -> String {
    match ty {
        Type::Array { .. } => format!("[{}]", strings(value).join(", ")),
        _ => scalar(value),
    }
}

/// `new T()`, `new T { A = 1 }`, or one initializer per line when there are several.
fn object(ty: &str, inits: &[String]) -> String {
    match inits.len() {
        0 => format!("new {ty}()"),
        1 => format!("new {ty} {{ {} }}", inits[0]),
        _ => {
            let mut out = format!("new {ty}\n    {{\n");
            for i in inits {
                let _ = writeln!(out, "        {i},");
            }
            out.push_str("    }");
            out
        }
    }
}
