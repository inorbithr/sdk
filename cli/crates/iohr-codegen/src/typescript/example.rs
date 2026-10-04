//! One operation's example in TypeScript, on `@inorbithr/sdk` (see [`crate::examples`]).

use std::fmt::Write as _;

use heck::{ToLowerCamelCase as _, ToUpperCamelCase as _};
use serde_json::Value;

use super::{TsNaming, key};
use crate::context::Naming as _;
use crate::examples::{Call, Flow, client, scalar, strings};
use crate::ir::Type;

/// The example of `call`.
pub(crate) fn example(call: &Call<'_>) -> String {
    let naming = TsNaming;
    let c = client::TYPESCRIPT;
    let mut names: Vec<&str> = c.imports.to_vec();
    names.push("ApiError");
    names.sort_unstable();
    names.dedup();
    let mut out = format!(
        "import {{ {} }} from \"@inorbithr/sdk\";\n\n",
        names.join(", ")
    );
    for line in c.lines {
        let _ = writeln!(out, "{line}");
    }
    out.push_str("\ntry {\n");
    let receiver = match call.tag {
        Some(tag) => format!("{}.{}", client::VAR, naming.method_name(tag)),
        None => client::VAR.to_owned(),
    };
    let method = match &call.flow {
        Flow::Pages(_) => format!("all{}", call.op.name.to_upper_camel_case()),
        _ => naming.method_name(&call.op.name),
    };
    let args = arguments(call, "  ");
    match &call.flow {
        Flow::Unary => {
            let _ = write!(
                out,
                "  const {{ value }} = await {receiver}.{method}({args});\n  console.log(value);\n"
            );
        }
        Flow::Pages(_) | Flow::Stream => {
            let item = call.item().to_lower_camel_case();
            let _ = write!(
                out,
                "  for await (const {item} of {receiver}.{method}({args})) {{\n    console.log({item});\n  }}\n"
            );
        }
    }
    out.push_str(
        "} catch (e) {\n  if (!(e instanceof ApiError)) throw e;\n  console.error(`${e.code}: ${e.problem} (request id ${e.raw.requestId})`);\n}\n",
    );
    out
}

/// The arguments: the path values, the query object (when there is one to give, or a
/// body follows it), the body.
fn arguments(call: &Call<'_>, indent: &str) -> String {
    let mut args: Vec<String> = call
        .path
        .iter()
        .map(|v| scalar(&Value::from(v.as_str())))
        .collect();
    if call.op.params_type.is_some() && (!call.query.is_empty() || call.body.is_some()) {
        let fields: Vec<String> = call
            .query
            .iter()
            .map(|(name, ty, value)| format!("{}: {}", key(name), literal(ty, value)))
            .collect();
        args.push(object(&fields, indent));
    }
    if let Some((_, fields)) = &call.body {
        let fields: Vec<String> = fields
            .iter()
            .map(|f| format!("{}: {}", key(&f.name), literal(&f.ty, &f.value)))
            .collect();
        args.push(object(&fields, indent));
    }
    args.join(", ")
}

fn literal(ty: &Type, value: &Value) -> String {
    match ty {
        Type::Array { .. } => format!("[{}]", strings(value).join(", ")),
        _ => scalar(value),
    }
}

/// `{}`, `{ a: 1 }`, or one field per line when there are several.
fn object(fields: &[String], indent: &str) -> String {
    match fields.len() {
        0 => "{}".into(),
        1 => format!("{{ {} }}", fields[0]),
        _ => {
            let mut out = "{\n".to_owned();
            for f in fields {
                let _ = writeln!(out, "{indent}  {f},");
            }
            let _ = write!(out, "{indent}}}");
            out
        }
    }
}
