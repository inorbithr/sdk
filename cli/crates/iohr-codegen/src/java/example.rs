//! One operation's example in Java, on `hr.inorbit:inorbit-sdk` (see
//! [`crate::examples`]): the imports and the statements, as they go in a method.

use std::fmt::Write as _;

use heck::ToLowerCamelCase as _;
use serde_json::Value;

use super::JavaNaming;
use crate::context::Naming as _;
use crate::examples::{Call, Flow, client, scalar, strings};
use crate::ir::Type;

/// The package the runtime's public surface is generated into.
const SURFACE: &str = "hr.inorbit.sdk.generated";

/// The example of `call`.
pub(crate) fn example(call: &Call<'_>) -> String {
    let naming = JavaNaming;
    let c = client::JAVA;
    let mut imports: Vec<String> = c.imports.iter().map(|s| (*s).to_owned()).collect();
    imports.push("hr.inorbit.sdk.errors.ApiException".into());
    let args = arguments(call, &mut imports);
    imports.sort_unstable();
    imports.dedup();
    let mut out = String::new();
    for i in &imports {
        let _ = writeln!(out, "import {i};");
    }
    out.push('\n');
    for line in c.lines {
        let _ = writeln!(out, "{line}");
    }
    out.push_str("try {\n");
    let receiver = match call.tag {
        Some(tag) => format!("{}.{}()", client::VAR, naming.method_name(tag)),
        None => client::VAR.to_owned(),
    };
    let method = naming.method_name(&call.op.name);
    match &call.flow {
        Flow::Unary => {
            let _ = write!(
                out,
                "    var value = {receiver}.{method}({args}).value();\n    System.out.println(value);\n"
            );
        }
        Flow::Pages(_) => {
            let item = escape(&call.item().to_lower_camel_case());
            let all = format!("all{}", naming.type_name(&call.op.name));
            let _ = write!(
                out,
                "    for (var {item} : {receiver}.{all}({args})) {{\n        System.out.println({item});\n    }}\n"
            );
        }
        Flow::Stream => {
            let _ = write!(
                out,
                "    try (var stream = {receiver}.{method}({args})) {{\n        for (var event : stream) {{\n            System.out.println(event);\n        }}\n    }}\n"
            );
        }
    }
    out.push_str(
        "} catch (ApiException e) {\n    System.err.println(e.code() + \": \" + e.problem() + \" (request id \" + e.raw().requestId() + \")\");\n}\n",
    );
    out
}

fn escape(name: &str) -> String {
    JavaNaming.escape(name.to_owned())
}

/// The path values, the query parameters (the overload without them when none is
/// required), the body as its builder.
fn arguments(call: &Call<'_>, imports: &mut Vec<String>) -> String {
    let naming = JavaNaming;
    let mut args: Vec<String> = call
        .path
        .iter()
        .map(|v| scalar(&Value::from(v.as_str())))
        .collect();
    if let Some(params) = &call.op.params_type
        && (!call.query.is_empty() || call.body.is_some())
    {
        imports.push(format!("{SURFACE}.{params}"));
        let setters: Vec<String> = call
            .query
            .iter()
            .map(|(name, ty, value)| {
                format!(
                    ".{}({})",
                    naming.field_name(name),
                    literal(ty, value, imports)
                )
            })
            .collect();
        args.push(builder(params, &setters));
    }
    if let Some((model, fields)) = &call.body {
        imports.push(format!("{SURFACE}.{model}"));
        let setters: Vec<String> = fields
            .iter()
            .map(|f| {
                format!(
                    ".{}({})",
                    naming.field_name(&f.name),
                    literal(&f.ty, &f.value, imports)
                )
            })
            .collect();
        args.push(builder(model, &setters));
    }
    args.join(", ")
}

fn literal(ty: &Type, value: &Value, imports: &mut Vec<String>) -> String {
    match ty {
        Type::Array { .. } => {
            imports.push("java.util.List".into());
            format!("List.of({})", strings(value).join(", "))
        }
        _ => scalar(value),
    }
}

/// `Type.builder().build()`, one setter per line when there are several.
fn builder(ty: &str, setters: &[String]) -> String {
    match setters.len() {
        0 => format!("{ty}.builder().build()"),
        1 => format!("{ty}.builder(){}.build()", setters[0]),
        _ => {
            let mut out = format!("{ty}.builder()\n");
            for s in setters {
                let _ = writeln!(out, "            {s}");
            }
            out.push_str("            .build()");
            out
        }
    }
}
