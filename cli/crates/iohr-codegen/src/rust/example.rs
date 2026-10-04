//! One operation's example in Rust, on `inorbithr` (see [`crate::examples`]). The
//! program's `run` returns the error with `?` and `main` reports it, the API's code and
//! request id first.

use std::fmt::Write as _;

use heck::{ToSnakeCase as _, ToUpperCamelCase as _};
use serde_json::Value;

use crate::examples::{Call, Flow, client, object_fields, scalar, strings};
use crate::ir::Type;

/// Words typify suffixes with `_` in a field name.
const KEYWORDS: &[&str] = &[
    "as", "async", "await", "box", "break", "const", "continue", "crate", "dyn", "else", "enum",
    "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
    "mut", "pub", "ref", "return", "self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "yield",
];

/// The example of `call`.
pub(crate) fn example(call: &Call<'_>) -> String {
    let c = client::RUST;
    let mut public: Vec<String> = vec!["Surface as _".into()];
    if let Some(p) = &call.op.params_type {
        public.push(p.clone());
    }
    if let Some((model, _)) = &call.body {
        public.push(model.to_upper_camel_case());
    }
    public.sort_by_key(|s| s.to_lowercase());
    let mut uses: Vec<String> = c.imports.iter().map(|s| (*s).to_owned()).collect();
    uses.push("inorbithr::Error".into());
    uses.push(format!("inorbithr::public::{{{}}}", public.join(", ")));
    let mut out = String::new();
    for u in group(uses) {
        let _ = writeln!(out, "use {u};");
    }
    out.push_str("\n#[tokio::main]\nasync fn main() {\n    if let Err(err) = run().await {\n        match err {\n            Error::Api(e) => eprintln!(\"{}: {} (request id {})\", e.code, e.message, e.raw.request_id),\n            e => eprintln!(\"{e}\"),\n        }\n        std::process::exit(1);\n    }\n}\n\nasync fn run() -> Result<(), Error> {\n");
    for line in c.lines {
        let _ = writeln!(out, "    {line}");
    }
    let receiver = match call.tag {
        Some(tag) => format!("{}.{}()", client::VAR, tag.to_snake_case()),
        None => client::VAR.to_owned(),
    };
    let method = &call.op.name;
    let args = arguments(call);
    match &call.flow {
        Flow::Unary => {
            let _ = write!(
                out,
                "    let res = {receiver}.{method}({args}).await?;\n    println!(\"{{:?}}\", res.value);\n"
            );
        }
        Flow::Pages(_) => {
            let item = call.item();
            let _ = write!(
                out,
                "    let mut pages = {receiver}.all_{method}({args});\n    while let Some({item}) = pages.next().await {{\n        println!(\"{{:?}}\", {item}?);\n    }}\n"
            );
        }
        Flow::Stream => {
            let _ = write!(
                out,
                "    let mut events = {receiver}.{method}({args}).await?;\n    while let Some(event) = events.next().await {{\n        println!(\"{{:?}}\", event?);\n    }}\n"
            );
        }
    }
    out.push_str("    Ok(())\n}\n");
    out
}

/// `use` paths merged by crate root the way rustfmt sorts them.
fn group(mut uses: Vec<String>) -> Vec<String> {
    uses.sort();
    let (root, rest): (Vec<String>, Vec<String>) = uses
        .into_iter()
        .partition(|u| u.starts_with("inorbithr::") && !u.starts_with("inorbithr::public"));
    let mut out = Vec::new();
    if !root.is_empty() {
        let names: Vec<&str> = root.iter().map(|u| &u[11..]).collect();
        out.push(if names.len() == 1 {
            format!("inorbithr::{}", names[0])
        } else {
            format!("inorbithr::{{{}}}", names.join(", "))
        });
    }
    out.extend(rest);
    out.sort();
    out
}

/// The path values, a reference to the query struct, a reference to the body.
fn arguments(call: &Call<'_>) -> String {
    let mut args: Vec<String> = call
        .path
        .iter()
        .map(|v| scalar(&Value::from(v.as_str())))
        .collect();
    if let Some(params) = &call.op.params_type {
        let fields: Vec<String> = call
            .query
            .iter()
            .map(|(name, ty, value)| format!("{}: Some({})", query_field(name), owned(ty, value)))
            .collect();
        args.push(format!("&{}", strukt(params, &fields)));
    }
    if let Some((model, fields)) = &call.body {
        let all = object_fields(call.models, model);
        let fields: Vec<String> = fields
            .iter()
            .map(|f| {
                let required = all
                    .iter()
                    .any(|x| x.name == f.name && x.required && !x.nullable);
                let value = owned(&f.ty, &f.value);
                let value = if required || matches!(f.ty, Type::Array { .. }) {
                    value
                } else {
                    format!("Some({value})")
                };
                format!("{}: {value}", model_field(&f.name))
            })
            .collect();
        args.push(format!(
            "&{}",
            strukt(&model.to_upper_camel_case(), &fields)
        ));
    }
    args.join(", ")
}

/// A query struct's field, as the surface spells it.
fn query_field(wire: &str) -> String {
    let snake = wire.to_snake_case();
    if ["type", "ref", "self", "match", "loop", "mod"].contains(&snake.as_str()) {
        format!("r#{snake}")
    } else {
        snake
    }
}

/// A model's field, as typify spells it.
fn model_field(wire: &str) -> String {
    let snake = wire.to_snake_case();
    if KEYWORDS.contains(&snake.as_str()) {
        format!("{snake}_")
    } else {
        snake
    }
}

fn owned(ty: &Type, value: &Value) -> String {
    match ty {
        Type::Array { .. } => {
            let items: Vec<String> = strings(value)
                .iter()
                .map(|s| format!("{s}.into()"))
                .collect();
            format!("vec![{}]", items.join(", "))
        }
        Type::String | Type::Enum { .. } => format!("{}.into()", scalar(value)),
        _ => scalar(value),
    }
}

/// `T::default()`, or the fields set and the rest defaulted.
fn strukt(ty: &str, fields: &[String]) -> String {
    if fields.is_empty() {
        return format!("{ty}::default()");
    }
    let mut out = format!("{ty} {{\n");
    for f in fields {
        let _ = writeln!(out, "        {f},");
    }
    out.push_str("        ..Default::default()\n    }");
    out
}
