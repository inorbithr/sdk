//! One operation's example in Swift, on the `InOrbit` package (see [`crate::examples`]):
//! statements in an `async throws` context, as a command-line tool's `main` writes them.

use std::fmt::Write as _;

use heck::{ToLowerCamelCase as _, ToUpperCamelCase as _};
use serde_json::Value;

use super::SwiftNaming;
use crate::context::Naming as _;
use crate::examples::{Call, Flow, client, scalar, strings};
use crate::ir::Type;

/// The example of `call`.
pub(crate) fn example(call: &Call<'_>) -> String {
    let naming = SwiftNaming;
    let c = client::SWIFT;
    let mut out = String::new();
    for i in c.imports {
        let _ = writeln!(out, "import {i}");
    }
    out.push('\n');
    for line in c.lines {
        let _ = writeln!(out, "{line}");
    }
    out.push_str("do {\n");
    let receiver = match call.tag {
        Some(tag) => format!("{}.{}", client::VAR, naming.method_name(tag)),
        None => client::VAR.to_owned(),
    };
    let args = arguments(call);
    match &call.flow {
        Flow::Unary => {
            let _ = write!(
                out,
                "    let value = try await {receiver}.{}({args}).value\n    print(value)\n",
                naming.method_name(&call.op.name)
            );
        }
        Flow::Pages(_) => {
            let item = naming.escape(call.item().to_lower_camel_case());
            let _ = write!(
                out,
                "    for try await {item} in {receiver}.all{}({args}) {{\n        print({item})\n    }}\n",
                call.op.name.to_upper_camel_case()
            );
        }
        Flow::Stream => {
            let _ = write!(
                out,
                "    for try await event in {receiver}.{}({args}) {{\n        print(event)\n    }}\n",
                naming.method_name(&call.op.name)
            );
        }
    }
    out.push_str(
        "} catch let e as APIError {\n    print(\"\\(e.code): \\(e.problem) (request id \\(e.raw.requestId))\")\n}\n",
    );
    out
}

/// The path values (labelled), the query value (when one is required, or a body follows),
/// the body.
fn arguments(call: &Call<'_>) -> String {
    let naming = SwiftNaming;
    let mut args: Vec<String> = call
        .op
        .path_params
        .iter()
        .zip(&call.path)
        .map(|(p, v)| {
            format!(
                "{}: {}",
                naming.field_name(&p.name).trim_matches('`'),
                scalar(&Value::from(v.as_str()))
            )
        })
        .collect();
    if let Some(params) = &call.op.params_type
        && !call.query.is_empty()
    {
        let inits: Vec<String> = call
            .query
            .iter()
            .map(|(name, ty, value)| {
                format!(
                    "{}: {}",
                    naming.field_name(name).trim_matches('`'),
                    literal(ty, value)
                )
            })
            .collect();
        args.push(format!("{params}({})", inits.join(", ")));
    }
    if let Some((model, fields)) = &call.body {
        // The memberwise initialiser takes the fields in their declared (name) order.
        let mut fields: Vec<_> = fields.iter().collect();
        fields.sort_by(|a, b| a.name.cmp(&b.name));
        let inits: Vec<String> = fields
            .iter()
            .map(|f| {
                format!(
                    "{}: {}",
                    naming.field_name(&f.name).trim_matches('`'),
                    literal(&f.ty, &f.value)
                )
            })
            .collect();
        args.push(format!("body: {model}({})", inits.join(", ")));
    }
    args.join(", ")
}

fn literal(ty: &Type, value: &Value) -> String {
    match ty {
        Type::Array { .. } => format!("[{}]", strings(value).join(", ")),
        _ => scalar(value),
    }
}
