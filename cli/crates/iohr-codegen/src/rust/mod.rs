//! The Rust target: models through typify, and the profiles, markers and surface
//! through templates. The shape is the one a developer's crate can hold: handle types
//! local to the surface, one extension trait on `Client<P>`, and a marker trait per
//! operation implemented for the profiles whose cut holds it, so a call a profile may
//! not make does not compile.

mod models;

mod example;
pub(crate) use example::example;

use std::collections::BTreeSet;

use heck::{ToSnakeCase as _, ToUpperCamelCase as _};
use iohr_openapi::model::type_name;
use iohr_openapi::{Api, Method};
use minijinja::Environment;
use serde::Serialize;

use crate::context::{self, Naming, Segment};
use crate::files::Files;
use crate::ir::{self, Type};
use crate::language::{Language, Options};
use crate::target::{RenderError, Target};

/// The Rust target.
#[derive(Debug, Clone, Copy, Default)]
pub struct RustTarget;

/// What the templates see.
#[derive(Debug, Serialize)]
struct Context {
    header: String,
    runtime: String,
    in_crate: bool,
    api_version: String,
    profiles: Vec<ProfileCtx>,
    markers: Vec<MarkerCtx>,
    handles: Vec<HandleCtx>,
    flat: Vec<OpCtx>,
    params_structs: Vec<ParamsCtx>,
    /// Whether any operation pages, which imports the runtime's pager.
    paged: bool,
    /// Whether any operation streams, which imports the runtime's `EventStream`.
    streamed: bool,
}

#[derive(Debug, Serialize)]
struct ProfileCtx {
    name: String,
    type_name: String,
    env: String,
    is_public: bool,
}

#[derive(Debug, Serialize)]
struct MarkerCtx {
    name: String,
    line: String,
    profiles: Vec<String>,
}

#[derive(Debug, Serialize)]
struct HandleCtx {
    tag: String,
    type_name: String,
    method_name: String,
    ops: Vec<OpCtx>,
}

#[derive(Debug, Clone, Serialize)]
struct OpCtx {
    name: String,
    hook_name: String,
    marker: String,
    doc: String,
    line: String,
    method: String,
    path_literal: Option<String>,
    path_format: Option<String>,
    path_args: Vec<String>,
    params_type: Option<String>,
    query: Vec<QueryCtx>,
    body_type: Option<String>,
    response_type: String,
    scopes: Vec<String>,
    idempotent: bool,
    /// How the operation pages, when it does and takes no body (design.md §9).
    paging: Option<PagingCtx>,
    /// Whether the answer is a stream of events (design.md §7).
    stream: bool,
    /// The RPC a socket call names (`x-iohr-rpc`); empty for none.
    rpc: String,
    /// The path parameters' wire names, beside `path_args`.
    path_wire: Vec<String>,
}

/// What `all_<operation>` needs: the item type, the params field the token goes in,
/// and how to read the list and the next token off the answer.
#[derive(Debug, Clone, Serialize)]
struct PagingCtx {
    item: String,
    token_field: String,
    list: String,
    next: String,
}

#[derive(Debug, Clone, Serialize)]
struct QueryCtx {
    name: String,
    field: String,
    is_vec: bool,
}

#[derive(Debug, Serialize)]
struct ParamsCtx {
    type_name: String,
    op_line: String,
    fields: Vec<FieldCtx>,
}

#[derive(Debug, Serialize)]
struct FieldCtx {
    name: String,
    field: String,
    ty: String,
    doc: String,
}

const MOD_TEMPLATE: &str = include_str!("templates/mod.rs.j2");
const PROFILES_TEMPLATE: &str = include_str!("templates/profiles.rs.j2");
const OPS_TEMPLATE: &str = include_str!("templates/ops.rs.j2");
const SURFACE_TEMPLATE: &str = include_str!("templates/surface.rs.j2");

impl Target for RustTarget {
    const LANG: Language = Language::Rust;

    fn render(&self, api: &Api, options: &Options) -> Result<Files, RenderError> {
        let named = options.runtime_for(Self::LANG);
        let runtime = if options.in_package {
            "crate"
        } else {
            named.as_str()
        };
        let mut files = Files::new();
        let ctx = context(api, runtime, options.in_package, &mut files);

        let models = models::render(
            &api.schemas
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            runtime,
        )?;
        files.insert("models.rs", format!("// {}\n\n{models}", ctx.header));

        let mut env = Environment::new();
        env.set_keep_trailing_newline(true);
        for (name, source) in [
            ("mod.rs", MOD_TEMPLATE),
            ("profiles.rs", PROFILES_TEMPLATE),
            ("ops.rs", OPS_TEMPLATE),
            ("surface.rs", SURFACE_TEMPLATE),
        ] {
            env.add_template(name, source)
                .map_err(|e| RenderError::Template {
                    file: name,
                    reason: e.to_string(),
                })?;
        }
        for name in ["mod.rs", "profiles.rs", "ops.rs", "surface.rs"] {
            let text = env
                .get_template(name)
                .and_then(|t| t.render(&ctx))
                .map_err(|e| RenderError::Template {
                    file: name,
                    reason: e.to_string(),
                })?;
            // mod.rs is written as the template has it, rustfmt-clean, because it is
            // the one file a developer's formatter sees: the others are declared with
            // `#[rustfmt::skip]`. syn drops plain comments, so the header goes on after
            // formatting.
            let formatted = if name == "mod.rs" {
                syn::parse_str::<syn::File>(&text).map_err(|e| RenderError::Syntax {
                    file: "mod.rs",
                    reason: e.to_string(),
                })?;
                text
            } else {
                format_rust(name, &text)?
            };
            files.insert(name, format!("// {}\n{formatted}", ctx.header));
        }
        Ok(files)
    }
}

/// Parses and reformats rendered code, so a template's whitespace never shows and a
/// template that renders something unparsable is caught here, not by the developer.
fn format_rust(file: &str, text: &str) -> Result<String, RenderError> {
    let parsed: syn::File = syn::parse_str(text).map_err(|e| RenderError::Syntax {
        file: match file {
            "mod.rs" => "mod.rs",
            "profiles.rs" => "profiles.rs",
            "ops.rs" => "ops.rs",
            _ => "surface.rs",
        },
        reason: format!("{e} in:\n{text}"),
    })?;
    Ok(prettyplease::unparse(&parsed))
}

/// How Rust spells names: the model's own `snake_case`, and `r#` before a keyword.
struct RustNaming;

impl Naming for RustNaming {
    const RESERVED: &'static [&'static str] = &["type", "ref", "self", "match", "loop", "mod"];

    fn escape(&self, ident: String) -> String {
        if Self::RESERVED.contains(&ident.as_str()) {
            format!("r#{ident}")
        } else {
            ident
        }
    }
}

fn context(api: &Api, runtime: &str, in_crate: bool, files: &mut Files) -> Context {
    let surface = context::surface(api);
    for note in &surface.notes {
        files.note(note.clone());
    }
    let mut params_structs = Vec::new();
    let models = context::models(api);
    let mut op = |o: &context::Op| op_context(o, &models, &mut params_structs);
    let flat: Vec<OpCtx> = surface.flat.iter().map(&mut op).collect();
    let handles: Vec<HandleCtx> = surface
        .handles
        .iter()
        .map(|h| HandleCtx {
            tag: h.tag.clone(),
            type_name: h.tag.to_upper_camel_case(),
            method_name: h.tag.to_snake_case(),
            ops: h.ops.iter().map(&mut op).collect(),
        })
        .collect();
    // The structs were pushed in handle order then flat order; the templates list
    // them in `METHOD /path` order, as before.
    params_structs.sort_by(|a: &ParamsCtx, b: &ParamsCtx| a.op_line.cmp(&b.op_line));
    Context {
        header: surface.header,
        runtime: runtime.to_owned(),
        in_crate,
        api_version: surface.api_version,
        profiles: surface
            .profiles
            .iter()
            .map(|p| ProfileCtx {
                name: p.name.clone(),
                type_name: type_name(&p.name),
                env: p.env.clone(),
                is_public: p.is_public,
            })
            .collect(),
        markers: surface
            .markers
            .iter()
            .map(|m| MarkerCtx {
                name: m.name.clone(),
                line: m.line.clone(),
                profiles: m.profiles.iter().map(|p| type_name(p)).collect(),
            })
            .collect(),
        paged: handles
            .iter()
            .any(|h: &HandleCtx| h.ops.iter().any(|o| o.paging.is_some())),
        streamed: handles
            .iter()
            .flat_map(|h: &HandleCtx| h.ops.iter())
            .chain(flat.iter())
            .any(|o| o.stream),
        handles,
        flat,
        params_structs,
    }
}

fn op_context(
    op: &context::Op,
    models: &[ir::Model],
    params_structs: &mut Vec<ParamsCtx>,
) -> OpCtx {
    let naming = RustNaming;
    let (path_literal, path_format, path_args) = if op.path_params.is_empty() {
        (Some(op.path.clone()), None, Vec::new())
    } else {
        let format: String = op
            .segments
            .iter()
            .map(|s| match s {
                Segment::Literal { text } => text.as_str(),
                Segment::Param { .. } => "{}",
            })
            .collect();
        (
            None,
            Some(format),
            op.path_params
                .iter()
                .map(|p| p.name.to_snake_case())
                .collect(),
        )
    };
    if let Some(type_name) = &op.params_type {
        params_structs.push(ParamsCtx {
            type_name: type_name.clone(),
            op_line: op.line.clone(),
            fields: op
                .query
                .iter()
                .map(|p| FieldCtx {
                    name: p.name.clone(),
                    field: naming.field_name(&p.name),
                    ty: rust_type(&p.ty),
                    doc: p.doc.clone(),
                })
                .collect(),
        });
    }
    OpCtx {
        name: op.name.clone(),
        hook_name: op.hook_name.clone(),
        marker: op.marker.clone(),
        doc: op.doc.clone(),
        line: op.line.clone(),
        method: match op.method {
            Method::Get => "Get",
            Method::Post => "Post",
            Method::Put => "Put",
            Method::Patch => "Patch",
            Method::Delete => "Delete",
            Method::Head => "Head",
        }
        .to_owned(),
        path_literal,
        path_format,
        path_args,
        params_type: op.params_type.clone(),
        query: op
            .query
            .iter()
            .map(|p| QueryCtx {
                name: p.name.clone(),
                field: naming.field_name(&p.name),
                is_vec: matches!(p.ty, Type::Array { .. }),
            })
            .collect(),
        body_type: op.body.clone(),
        response_type: op
            .response
            .clone()
            .unwrap_or_else(|| "::serde_json::Value".to_owned()),
        scopes: op.scopes.clone(),
        idempotent: op.idempotent_override,
        paging: if op.stream {
            None
        } else {
            paging_context(op, models)
        },
        stream: op.stream,
        rpc: op.rpc.clone(),
        path_wire: op.path_params.iter().map(|p| p.name.clone()).collect(),
    }
}

/// The paging of `op` for the template, when it pages and takes no body.
fn paging_context(op: &context::Op, models: &[ir::Model]) -> Option<PagingCtx> {
    let naming = RustNaming;
    if op.body.is_some() {
        return None;
    }
    let paging = context::paging(op, models)?;
    let response = op.response.as_deref()?;
    let ir::Shape::Object { fields } = &models.iter().find(|m| m.name == response)?.shape else {
        return None;
    };
    // A field typify reads as plain when the schema requires it and it is not nullable;
    // otherwise it is an `Option`, and an absent list or token reads as empty.
    let read = |wire: &str| -> Option<String> {
        let f = fields.iter().find(|f| f.name == wire)?;
        let name = naming.field_name(wire);
        Some(if f.required && !f.nullable {
            name
        } else {
            format!("{name}.unwrap_or_default()")
        })
    };
    let item = match &paging.item {
        Type::Ref { name } => name.clone(),
        other => rust_type(other),
    };
    Some(PagingCtx {
        item,
        token_field: naming.field_name(&paging.token_param),
        list: read(&paging.list_field)?,
        next: read(&paging.next_field)?,
    })
}

/// The Rust type of a query parameter.
fn rust_type(ty: &Type) -> String {
    match ty {
        Type::Integer { bits: 32 } => "i32".into(),
        Type::Integer { .. } => "i64".into(),
        Type::Number => "f64".into(),
        Type::Bool => "bool".into(),
        Type::Array { .. } => "::std::vec::Vec<::std::string::String>".into(),
        _ => "::std::string::String".into(),
    }
}

/// Every profile type a surface defines, for the compile test's negative case.
#[must_use]
pub fn profile_types(api: &Api) -> BTreeSet<String> {
    api.profiles.keys().map(|p| type_name(p)).collect()
}
