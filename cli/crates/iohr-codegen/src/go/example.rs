//! One operation's example in Go, on `github.com/inorbithr/sdk/go` (see
//! [`crate::examples`]). The program's `run` returns the error and `main` reports it,
//! the API's code and request id first.

use std::fmt::Write as _;

use serde_json::Value;

use super::{GoNaming, field_names};
use crate::context::Naming as _;
use crate::examples::{Call, Flow, client, object_fields, scalar, strings};
use crate::ir::Type;

/// The example of `call`.
pub(crate) fn example(call: &Call<'_>) -> String {
    let naming = GoNaming;
    let c = client::GO;
    let uses_models = call.body.is_some() || !call.query.is_empty();
    let mut imports: Vec<String> = c.imports.iter().map(|s| format!("{s:?}")).collect();
    imports.push("inorbit \"github.com/inorbithr/sdk/go\"".into());
    if uses_models {
        imports.push("\"github.com/inorbithr/sdk/go/public/models\"".into());
    }
    imports.sort_by(|a, b| unquoted(a).cmp(unquoted(b)));
    imports.dedup();
    let mut out = String::from(
        "package main\n\nimport (\n\t\"context\"\n\t\"errors\"\n\t\"fmt\"\n\t\"log\"\n\n",
    );
    for i in &imports {
        let _ = writeln!(out, "\t{i}");
    }
    out.push_str(")\n\nfunc main() {\n\tif err := run(context.Background()); err != nil {\n\t\tvar apiErr *inorbit.APIError\n\t\tif errors.As(err, &apiErr) {\n\t\t\tlog.Fatalf(\"%s: %s (request id %s)\", apiErr.Code, apiErr.Problem, apiErr.Raw.RequestID)\n\t\t}\n\t\tlog.Fatal(err)\n\t}\n}\n\nfunc run(ctx context.Context) error {\n");
    for line in c.lines {
        let _ = writeln!(out, "\t{line}");
    }
    let receiver = match call.tag {
        Some(tag) => format!("{}.{}()", client::VAR, naming.type_name(tag)),
        None => client::VAR.to_owned(),
    };
    let method = naming.method_name(&call.op.name);
    let args = arguments(call);
    match &call.flow {
        Flow::Unary => {
            let _ = write!(
                out,
                "\tres, err := {receiver}.{method}({args})\n\tif err != nil {{\n\t\treturn err\n\t}}\n\tfmt.Printf(\"%+v\\n\", res.Value)\n"
            );
        }
        Flow::Pages(_) => {
            let item = lower_camel(&call.item());
            let _ = write!(
                out,
                "\tfor {item}, err := range {receiver}.All{method}({args}) {{\n\t\tif err != nil {{\n\t\t\treturn err\n\t\t}}\n\t\tfmt.Printf(\"%+v\\n\", {item})\n\t}}\n"
            );
        }
        Flow::Stream => {
            let _ = write!(
                out,
                "\tfor event, err := range {receiver}.{method}({args}) {{\n\t\tif err != nil {{\n\t\t\treturn err\n\t\t}}\n\t\tfmt.Printf(\"%+v\\n\", *event)\n\t}}\n"
            );
        }
    }
    out.push_str("\treturn nil\n}\n");
    out
}

fn unquoted(import: &str) -> &str {
    import.rsplit(' ').next().unwrap_or(import)
}

fn lower_camel(snake: &str) -> String {
    let exported = GoNaming.type_name(snake);
    let mut chars = exported.chars();
    chars.next().map_or_else(String::new, |f| {
        f.to_ascii_lowercase().to_string() + chars.as_str()
    })
}

/// `ctx`, the path values, the query parameters (`nil` when none is required), the body.
fn arguments(call: &Call<'_>) -> String {
    let mut args = vec!["ctx".to_owned()];
    args.extend(call.path.iter().map(|v| scalar(&Value::from(v.as_str()))));
    if let Some(params) = &call.op.params_type {
        if call.query.is_empty() {
            args.push("nil".into());
        } else {
            let names = field_names(call.op.query.iter().map(|q| q.name.as_str()));
            let fields: Vec<(String, String)> = call
                .query
                .iter()
                .map(|(name, ty, value)| {
                    let at = call
                        .op
                        .query
                        .iter()
                        .position(|q| &q.name == name)
                        .unwrap_or(0);
                    (names[at].clone(), literal(ty, value, false))
                })
                .collect();
            let by_value = call.op.query.iter().any(|q| q.required);
            let amp = if by_value { "" } else { "&" };
            args.push(format!("{amp}models.{params}{}", composite(&fields)));
        }
    }
    if let Some((model, fields)) = &call.body {
        let all = object_fields(call.models, model);
        let names = field_names(all.iter().map(|f| f.name.as_str()));
        let filled: Vec<(String, String)> = fields
            .iter()
            .map(|f| {
                let at = all.iter().position(|x| x.name == f.name).unwrap_or(0);
                let pointer = all.get(at).is_some_and(|x| {
                    (!x.required || x.nullable) && !matches!(x.ty, Type::Array { .. })
                });
                (names[at].clone(), literal(&f.ty, &f.value, pointer))
            })
            .collect();
        args.push(format!(
            "models.{}{}",
            GoNaming.type_name(model),
            composite(&filled)
        ));
    }
    args.join(", ")
}

fn literal(ty: &Type, value: &Value, pointer: bool) -> String {
    let plain = match ty {
        Type::Array { .. } => format!("[]string{{{}}}", strings(value).join(", ")),
        Type::Integer { bits: 32 } => format!("int32({})", scalar(value)),
        Type::Integer { .. } => format!("int64({})", scalar(value)),
        _ => scalar(value),
    };
    if pointer {
        format!("inorbit.Ptr({plain})")
    } else {
        plain
    }
}

/// `{}`, `{A: 1}`, or one field per line with the values aligned, as gofmt writes it.
fn composite(fields: &[(String, String)]) -> String {
    match fields.len() {
        0 => "{}".into(),
        1 => format!("{{{}: {}}}", fields[0].0, fields[0].1),
        _ => {
            let width = fields.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
            let mut out = "{\n".to_owned();
            for (k, v) in fields {
                let pad = " ".repeat(width - k.len() + 1);
                let _ = writeln!(out, "\t\t{k}:{pad}{v},");
            }
            out.push_str("\t}");
            out
        }
    }
}
