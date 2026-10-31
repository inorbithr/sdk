//! One operation's example in Rust, on `inorbithr` (see [`crate::examples`]). The
//! program's `run` returns the error with `?` and `main` reports it, the API's code and
//! request id first.

use std::fmt::Write as _;

use heck::{ToSnakeCase as _, ToUpperCamelCase as _};
use serde_json::Value;

use crate::examples::{Call, Flow, client, scalar, strings};
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
    uses.push(format!("inorbithr::public::{{{}}}", public.join(", ")));
    let mut out = String::new();
    for u in group(uses) {
        let _ = writeln!(out, "use {u};");
    }
    out.push('\n');
    let receiver = match call.tag {
        Some(tag) => format!("{}.{}()", client::VAR, tag.to_snake_case()),
        None => client::VAR.to_owned(),
    };
    let method = &call.op.name;
    let args = arguments(call);
    let mut body = String::new();
    match &call.flow {
        Flow::Unary => {
            let _ = write!(
                body,
                "let res = {receiver}.{method}({args}).await?;\nprintln!(\"{{:?}}\", res.value);\n"
            );
        }
        Flow::Pages(_) => {
            let item = call.item();
            // The walk borrows the parameters, so they live in a variable.
            let (bind, args) = match args.split_once('&') {
                Some((path, params)) if call.op.params_type.is_some() => (
                    format!("let params = {params};\n"),
                    format!("{path}&params"),
                ),
                _ => (String::new(), args),
            };
            body.push_str(&bind);
            // The handle too.
            let receiver = match call.tag {
                Some(tag) => {
                    let name = tag.to_snake_case();
                    let _ = writeln!(body, "let {name} = {receiver};");
                    name
                }
                None => receiver.clone(),
            };
            let _ = write!(
                body,
                "let mut pages = {receiver}.all_{method}({args});\nwhile let Some({item}) = pages.next().await {{\n    println!(\"{{:?}}\", {item}?);\n}}\n"
            );
        }
        Flow::Stream => {
            let _ = write!(
                body,
                "let mut events = {receiver}.{method}({args}).await?;\nwhile let Some(event) = events.next().await {{\n    println!(\"{{:?}}\", event?);\n}}\n"
            );
        }
    }
    out.push_str(&c.frame(&body));
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
            .map(|(name, ty, value)| setter(&query_field(name), ty, value))
            .collect();
        args.push(format!("&{}", strukt(params, &fields)));
    }
    if let Some((model, fields)) = &call.body {
        let fields: Vec<String> = fields
            .iter()
            .map(|f| setter(&model_field(&f.name), &f.ty, &f.value))
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

/// `.with_<field>(value)`: the setter the surface gives every field of a struct that is
/// `#[non_exhaustive]` (a struct expression cannot name it from another crate).
fn setter(field: &str, ty: &Type, value: &Value) -> String {
    let field = field.strip_prefix("r#").unwrap_or(field);
    let value = match ty {
        Type::Array { .. } => {
            let items: Vec<String> = strings(value)
                .iter()
                .map(|s| format!("{s}.to_owned()"))
                .collect();
            format!("vec![{}]", items.join(", "))
        }
        _ => scalar(value),
    };
    format!("with_{field}({value})")
}

/// `T::default()` with the fields set through their setters.
fn strukt(ty: &str, fields: &[String]) -> String {
    let mut out = format!("{ty}::default()");
    for f in fields {
        let _ = write!(out, "\n    .{f}");
    }
    out
}
