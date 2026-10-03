//! The Rust target: models through typify, and the profiles, markers and surface
//! through templates. The shape is the one a developer's crate can hold: handle types
//! local to the surface, one extension trait on `Client<P>`, and a marker trait per
//! operation implemented for the profiles whose cut holds it, so a call a profile may
//! not make does not compile.

mod models;

use std::collections::{BTreeMap, BTreeSet};

use heck::{ToSnakeCase as _, ToUpperCamelCase as _};
use iohr_openapi::model::{env_name, type_name};
use iohr_openapi::{Api, Media, Method, Operation, ParamIn};
use minijinja::Environment;
use serde::Serialize;

use crate::files::Files;
use crate::target::{RenderError, Target};

/// Flags of the Rust target.
#[derive(Debug, Clone, clap::Args)]
pub struct RustOptions {
    /// The crate the surface runs on, as it is named in your `Cargo.toml`.
    #[arg(long, default_value = "inorbithr")]
    pub runtime: String,
    /// Generate the runtime's own public surface, which refers to it as `crate`.
    #[arg(long, hide = true)]
    pub in_crate: bool,
}

impl Default for RustOptions {
    fn default() -> Self {
        Self {
            runtime: "inorbithr".into(),
            in_crate: false,
        }
    }
}

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
    type Options = RustOptions;

    const LANG: &'static str = "rust";

    fn render(&self, api: &Api, options: &Self::Options) -> Result<Files, RenderError> {
        let runtime = if options.in_crate {
            "crate"
        } else {
            options.runtime.as_str()
        };
        let mut files = Files::new();
        let ctx = context(api, runtime, options.in_crate, &mut files);

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

fn context(api: &Api, runtime: &str, in_crate: bool, files: &mut Files) -> Context {
    let profiles: Vec<ProfileCtx> = api
        .profiles
        .keys()
        .map(|name| ProfileCtx {
            name: name.clone(),
            type_name: type_name(name),
            env: env_name(name),
            is_public: name == "public",
        })
        .collect();
    let markers = marker_names(api);
    let mut params_structs = Vec::new();
    let mut by_tag: BTreeMap<&str, Vec<OpCtx>> = BTreeMap::new();
    let mut flat = Vec::new();
    for op in api.operations.values() {
        match &op.response.media {
            Media::Json => {}
            Media::EventStream => {
                files.note(format!(
                    "{}: a stream (text/event-stream); streaming comes with the runtime's streaming milestone, so the operation is left out",
                    op.line()
                ));
                continue;
            }
            Media::Other(m) => {
                files.note(format!(
                    "{}: answers {m}, which the Rust target does not render; left out",
                    op.line()
                ));
                continue;
            }
        }
        let ctx = op_context(op, &markers, &mut params_structs);
        if op.service.is_some() {
            by_tag.entry(op.tag.as_str()).or_default().push(ctx);
        } else {
            flat.push(ctx);
        }
    }
    let handles = by_tag
        .into_iter()
        .map(|(tag, ops)| HandleCtx {
            tag: tag.to_owned(),
            type_name: tag.to_upper_camel_case(),
            method_name: tag.to_snake_case(),
            ops,
        })
        .collect();
    let marker_ctx = api
        .operations
        .values()
        .filter(|op| op.response.media == Media::Json)
        .map(|op| MarkerCtx {
            name: markers[&op.line()].clone(),
            line: op.line(),
            profiles: op.profiles.iter().map(|p| type_name(p)).collect(),
        })
        .collect();
    Context {
        header: crate::header(api),
        runtime: runtime.to_owned(),
        in_crate,
        api_version: api.api_version.clone(),
        profiles,
        markers: marker_ctx,
        handles,
        flat,
        params_structs,
    }
}

/// One marker trait per operation, named after the operation; when two tags share a
/// name, both take their tag as a prefix.
fn marker_names(api: &Api) -> BTreeMap<String, String> {
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    for op in api.operations.values() {
        *count.entry(op.name.to_upper_camel_case()).or_default() += 1;
    }
    api.operations
        .values()
        .map(|op| {
            let short = op.name.to_upper_camel_case();
            let name = if count[&short] > 1 {
                format!("{}{short}", op.tag.to_upper_camel_case())
            } else {
                short
            };
            (op.line(), name)
        })
        .collect()
}

fn op_context(
    op: &Operation,
    markers: &BTreeMap<String, String>,
    params_structs: &mut Vec<ParamsCtx>,
) -> OpCtx {
    let path_params: Vec<&str> = op
        .params
        .iter()
        .filter(|p| p.location == ParamIn::Path)
        .map(|p| p.name.as_str())
        .collect();
    let (path_literal, path_format, path_args) = if path_params.is_empty() {
        (Some(op.path.clone()), None, Vec::new())
    } else {
        let mut format = op.path.clone();
        for name in &path_params {
            format = format.replace(&format!("{{{name}}}"), "{}");
        }
        (
            None,
            Some(format),
            path_params.iter().map(|n| n.to_snake_case()).collect(),
        )
    };
    let query: Vec<&iohr_openapi::Param> = op
        .params
        .iter()
        .filter(|p| p.location == ParamIn::Query)
        .collect();
    let params_type = if query.is_empty() {
        None
    } else {
        let type_name = format!("{}Params", op.name.to_upper_camel_case());
        let type_name = match &op.service {
            Some(_) => format!("{}{type_name}", op.tag.to_upper_camel_case()),
            None => type_name,
        };
        params_structs.push(ParamsCtx {
            type_name: type_name.clone(),
            op_line: op.line(),
            fields: query
                .iter()
                .map(|p| FieldCtx {
                    name: p.name.clone(),
                    field: field_name(&p.name),
                    ty: rust_type(&p.schema),
                    doc: p.doc.clone(),
                })
                .collect(),
        });
        Some(type_name)
    };
    OpCtx {
        name: op.name.clone(),
        hook_name: match &op.service {
            Some(_) => format!("{}.{}", op.tag, op.name),
            None => op.name.clone(),
        },
        marker: markers[&op.line()].clone(),
        doc: op.doc.clone(),
        line: op.line(),
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
        params_type,
        query: query
            .iter()
            .map(|p| QueryCtx {
                name: p.name.clone(),
                field: field_name(&p.name),
                is_vec: p.schema.get("type") == Some(&serde_json::json!("array")),
            })
            .collect(),
        body_type: op.request_body.clone(),
        response_type: op
            .response
            .schema
            .clone()
            .unwrap_or_else(|| "::serde_json::Value".to_owned()),
        scopes: op.scopes.clone(),
        idempotent: op.idempotent && !op.method.is_idempotent(),
    }
}

/// Rust keywords a wire name may collide with.
const KEYWORDS: [&str; 6] = ["type", "ref", "self", "match", "loop", "mod"];

/// A query parameter's field: `snake_case`, with a Rust keyword escaped.
fn field_name(wire: &str) -> String {
    let name = wire.to_snake_case();
    if KEYWORDS.contains(&name.as_str()) {
        format!("r#{name}")
    } else {
        name
    }
}

/// The Rust type of a query parameter's schema.
fn rust_type(schema: &serde_json::Value) -> String {
    let ty = schema
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("string");
    let format = schema
        .get("format")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    match (ty, format) {
        ("integer", "int32") => "i32".into(),
        ("integer", _) => "i64".into(),
        ("number", _) => "f64".into(),
        ("boolean", _) => "bool".into(),
        ("array", _) => "::std::vec::Vec<::std::string::String>".into(),
        _ => "::std::string::String".into(),
    }
}

/// Every profile type a surface defines, for the compile test's negative case.
#[must_use]
pub fn profile_types(api: &Api) -> BTreeSet<String> {
    api.profiles.keys().map(|p| type_name(p)).collect()
}
