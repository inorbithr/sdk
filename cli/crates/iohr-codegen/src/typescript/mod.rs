//! The TypeScript target: models as interfaces and type aliases, the operations as
//! functions over the runtime's one request path, and one class per profile whose
//! handles hold only the operations its cut holds, so a call the profile may not make
//! is a type error. Plain JavaScript gets the same classes without the check.
//!
//! The runtime carries 64-bit integers as `bigint`; the wire carries them as decimal
//! strings. A model that holds one, directly or through another model, is listed in
//! `shapes`, which the runtime's `codegen.decode`/`encode` walk to convert at the
//! boundary and nowhere else.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use heck::ToLowerCamelCase as _;
use iohr_openapi::{Api, Method};

use crate::context::{self, Naming, Op, Segment};
use crate::files::Files;
use crate::ir::{self, Field, Model, Shape, Type};
use crate::language::{Language, Options};
use crate::target::{RenderError, Target};

/// The TypeScript target.
#[derive(Debug, Clone, Copy, Default)]
pub struct TypeScriptTarget;

/// How TypeScript spells names: `lowerCamel` methods and arguments, a reserved word
/// suffixed with `_`. Model fields keep their wire names, as the JSON has them.
struct TsNaming;

impl Naming for TsNaming {
    const RESERVED: &'static [&'static str] = &[
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "default",
        "delete",
        "do",
        "else",
        "enum",
        "export",
        "extends",
        "false",
        "finally",
        "for",
        "function",
        "if",
        "implements",
        "import",
        "in",
        "instanceof",
        "interface",
        "let",
        "new",
        "null",
        "package",
        "private",
        "protected",
        "public",
        "return",
        "static",
        "super",
        "switch",
        "this",
        "throw",
        "true",
        "try",
        "typeof",
        "var",
        "void",
        "while",
        "with",
        "yield",
    ];

    fn method_name(&self, snake: &str) -> String {
        self.escape(snake.to_lower_camel_case())
    }

    fn field_name(&self, wire: &str) -> String {
        self.escape(wire.to_lower_camel_case())
    }
}

impl Target for TypeScriptTarget {
    const LANG: Language = Language::TypeScript;

    fn render(&self, api: &Api, options: &Options) -> Result<Files, RenderError> {
        let runtime = match (&options.runtime, options.in_package) {
            (Some(r), _) => r.clone(),
            (None, true) => "../runtime.js".to_owned(),
            (None, false) => Self::LANG.default_runtime().to_owned(),
        };
        let surface = context::surface(api);
        let models = context::models(api);
        let needs = needs_conversion(&models);
        let mut files = Files::new();
        for note in &surface.notes {
            files.note(note.clone());
        }
        let header = format!("// {}\n", surface.header);
        files.insert(
            "models.ts",
            format!("{header}{}", render_models(&models, &needs, &runtime)),
        );
        files.insert(
            "operations.ts",
            format!("{header}{}", render_operations(&surface, &needs, &runtime)),
        );
        files.insert(
            "profiles.ts",
            format!("{header}{}", render_profiles(&surface, &runtime)),
        );
        files.insert(
            "index.ts",
            format!("{header}{}", render_index(&surface, &runtime)),
        );
        Ok(files)
    }
}

/// The models whose values hold a 64-bit integer, directly or through another model:
/// the ones the runtime converts at the boundary.
fn needs_conversion(models: &[Model]) -> BTreeSet<String> {
    let mut needs = BTreeSet::new();
    loop {
        let before = needs.len();
        for m in models.iter().filter(|m| !m.runtime) {
            let hit = match &m.shape {
                Shape::Object { fields } => fields.iter().any(|f| converts(&f.ty, &needs)),
                Shape::Alias { ty } => converts(ty, &needs),
                _ => false,
            };
            if hit {
                needs.insert(m.name.clone());
            }
        }
        if needs.len() == before {
            return needs;
        }
    }
}

fn converts(ty: &Type, needs: &BTreeSet<String>) -> bool {
    match ty {
        Type::Int64 => true,
        Type::Ref { name } => needs.contains(name),
        Type::Array { item } => converts(item, needs),
        Type::Map { value } => converts(value, needs),
        _ => false,
    }
}

/// The conversion a value of `ty` needs, as the runtime's `Shape` literal.
fn shape_literal(ty: &Type, needs: &BTreeSet<String>) -> Option<String> {
    match ty {
        Type::Int64 => Some("\"i64\"".into()),
        Type::Ref { name } if needs.contains(name) => Some(format!("{{ ref: \"{name}\" }}")),
        Type::Array { item } => shape_literal(item, needs).map(|s| format!("{{ array: {s} }}")),
        Type::Map { value } => shape_literal(value, needs).map(|s| format!("{{ map: {s} }}")),
        _ => None,
    }
}

/// A type expression.
fn ts_type(ty: &Type) -> String {
    match ty {
        Type::Ref { name } => name.clone(),
        Type::Int64 => "Int64".into(),
        Type::Integer { .. } | Type::Number => "number".into(),
        Type::String => "string".into(),
        Type::Bool => "boolean".into(),
        Type::Array { item } => {
            let inner = ts_type(item);
            if matches!(**item, Type::Enum { .. }) {
                format!("({inner})[]")
            } else {
                format!("{inner}[]")
            }
        }
        Type::Map { value } => format!("Record<string, {}>", ts_type(value)),
        Type::Enum { values } => values
            .iter()
            .map(|v| format!("\"{v}\""))
            .collect::<Vec<_>>()
            .join(" | "),
        _ => "unknown".into(),
    }
}

/// A doc comment, or nothing.
fn doc(text: &str, indent: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    format!(
        "{indent}/** {} */\n",
        text.replace("*/", "*\\/").replace('\n', " ")
    )
}

/// A property key: the wire name, quoted when it is not an identifier.
fn key(name: &str) -> String {
    let ident = name.chars().enumerate().all(|(i, c)| {
        c == '_' || c == '$' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit())
    });
    if ident && !name.is_empty() {
        name.to_owned()
    } else {
        format!("\"{name}\"")
    }
}

/// The types a set of type expressions refers to.
fn refs(ty: &Type, out: &mut BTreeSet<String>) {
    match ty {
        Type::Ref { name } => {
            out.insert(name.clone());
        }
        Type::Array { item } => refs(item, out),
        Type::Map { value } => refs(value, out),
        _ => {}
    }
}

fn render_models(models: &[Model], needs: &BTreeSet<String>, runtime: &str) -> String {
    let mut used_runtime = BTreeSet::new();
    let mut body = String::new();
    for m in models.iter().filter(|m| !m.runtime) {
        let mut mentioned = BTreeSet::new();
        body.push('\n');
        body.push_str(&doc(&m.doc, ""));
        match &m.shape {
            Shape::Object { fields } => {
                if fields.is_empty() {
                    let _ = writeln!(body, "export type {} = Record<string, never>;", m.name);
                } else {
                    let _ = writeln!(body, "export interface {} {{", m.name);
                    for f in fields {
                        refs(&f.ty, &mut mentioned);
                        body.push_str(&field(f));
                    }
                    body.push_str("}\n");
                }
            }
            Shape::Enum { values } => {
                let _ = writeln!(
                    body,
                    "export type {} = {};",
                    m.name,
                    ts_type(&Type::Enum {
                        values: values.clone()
                    })
                );
            }
            Shape::Union { variants, .. } => {
                for v in variants {
                    refs(v, &mut mentioned);
                }
                let joined: Vec<String> = variants.iter().map(ts_type).collect();
                let _ = writeln!(body, "export type {} = {};", m.name, joined.join(" | "));
            }
            Shape::Alias { ty } => {
                refs(ty, &mut mentioned);
                let _ = writeln!(body, "export type {} = {};", m.name, ts_type(ty));
            }
        }
        used_runtime.extend(
            mentioned
                .into_iter()
                .filter(|n| ir::RUNTIME_TYPES.contains(&n.as_str())),
        );
        if contains_int64(&m.shape) {
            used_runtime.insert("Int64".to_owned());
        }
    }
    let mut out = String::new();
    used_runtime.insert("codegen".to_owned());
    let imports: Vec<String> = used_runtime
        .iter()
        .map(|n| {
            if n == "codegen" {
                n.clone()
            } else {
                format!("type {n}")
            }
        })
        .collect();
    let _ = writeln!(
        out,
        "import {{ {} }} from \"{runtime}\";",
        imports.join(", ")
    );
    out.push_str(&body);
    out.push_str(
        "\n/** Where each model holds a 64-bit integer, for the runtime to convert. */\nexport const shapes: codegen.Shapes = {\n",
    );
    for m in models.iter().filter(|m| needs.contains(&m.name)) {
        let entries: Vec<String> = match &m.shape {
            Shape::Object { fields } => fields
                .iter()
                .filter_map(|f| {
                    shape_literal(&f.ty, needs).map(|s| format!("{}: {s}", key(&f.name)))
                })
                .collect(),
            _ => Vec::new(),
        };
        let _ = writeln!(out, "  {}: {{ {} }},", m.name, entries.join(", "));
    }
    out.push_str("};\n");
    out
}

fn contains_int64(shape: &Shape) -> bool {
    fn has(ty: &Type) -> bool {
        match ty {
            Type::Int64 => true,
            Type::Array { item } => has(item),
            Type::Map { value } => has(value),
            _ => false,
        }
    }
    match shape {
        Shape::Object { fields } => fields.iter().any(|f| has(&f.ty)),
        Shape::Alias { ty } => has(ty),
        _ => false,
    }
}

fn field(f: &Field) -> String {
    let mut ty = ts_type(&f.ty);
    if f.nullable {
        ty.push_str(" | null");
    }
    let optional = if f.required { "" } else { "?" };
    format!("{}  {}{optional}: {ty};\n", doc(&f.doc, "  "), key(&f.name))
}

/// The function an operation becomes in `operations.ts`: named after its marker, which
/// is unique.
fn function_name(op: &Op) -> String {
    op.marker.to_lower_camel_case()
}

fn scopes_note(op: &Op) -> String {
    if op.scopes.is_empty() {
        return String::new();
    }
    let list: Vec<String> = op.scopes.iter().map(|s| format!("`{s}`")).collect();
    format!(
        "; needs scope{} {}",
        if op.scopes.len() > 1 { "s" } else { "" },
        list.join(", ")
    )
}

fn op_doc(op: &Op, indent: &str) -> String {
    let mut text = format!("`{}`{}.", op.line, scopes_note(op));
    if !op.doc.is_empty() {
        let _ = write!(text, " {}", op.doc);
    }
    doc(&text, indent)
}

/// The arguments an operation takes, as a declaration and as a call.
fn arguments(op: &Op) -> (String, String) {
    let naming = TsNaming;
    let mut decl = vec!["client: Client".to_owned()];
    let mut call = vec!["this.client".to_owned()];
    for p in &op.path_params {
        let name = naming.field_name(&p.name);
        decl.push(format!("{name}: string"));
        call.push(name);
    }
    if let Some(t) = &op.params_type {
        if op.query.iter().any(|q| q.required) {
            decl.push(format!("params: {t}"));
        } else {
            decl.push(format!("params: {t} = {{}}"));
        }
        call.push("params".into());
    }
    if let Some(body) = &op.body {
        decl.push(format!("body: {body}"));
        call.push("body".into());
    }
    decl.push("options?: CallOptions".into());
    call.push("options".into());
    (decl.join(", "), call.join(", "))
}

fn method(m: Method) -> &'static str {
    m.as_str()
}

fn render_operations(
    surface: &context::Surface,
    needs: &BTreeSet<String>,
    runtime: &str,
) -> String {
    let mut sorted: Vec<&Op> = surface
        .handles
        .iter()
        .flat_map(|h| h.ops.iter())
        .chain(surface.flat.iter())
        .collect();
    sorted.sort_by(|a, b| a.line.cmp(&b.line));
    let mut models = BTreeSet::new();
    let mut uses_shapes = false;
    let mut body = String::new();
    for op in sorted {
        if let Some(t) = &op.params_type {
            body.push_str(&params_interface(t, op));
        }
        models.extend(op.body.iter().cloned());
        models.extend(op.response.iter().cloned());
        body.push_str(&operation_function(op, needs, &mut uses_shapes));
    }
    let mut out = format!(
        "import {{ type CallOptions, type Client, codegen, type Response }} from \"{runtime}\";\n"
    );
    let mut imports: Vec<String> = models.into_iter().map(|m| format!("type {m}")).collect();
    if uses_shapes {
        imports.push("shapes".into());
    }
    imports.sort_by(|a, b| {
        a.trim_start_matches("type ")
            .cmp(b.trim_start_matches("type "))
    });
    if !imports.is_empty() {
        let _ = writeln!(
            out,
            "import {{ {} }} from \"./models.js\";",
            imports.join(", ")
        );
    }
    out.push_str(&body);
    out
}

fn params_interface(type_name: &str, op: &Op) -> String {
    let mut out = format!(
        "\n/** The query parameters of `{}`. */\nexport interface {type_name} {{\n",
        op.line
    );
    for q in &op.query {
        let optional = if q.required { "" } else { "?" };
        let _ = writeln!(
            out,
            "{}  {}{optional}: {};",
            doc(&q.doc, "  "),
            key(&q.name),
            ts_type(&q.ty)
        );
    }
    out.push_str("}\n");
    out
}

/// One operation as a function over the runtime's request path.
fn operation_function(op: &Op, needs: &BTreeSet<String>, uses_shapes: &mut bool) -> String {
    let naming = TsNaming;
    let (decl, _) = arguments(op);
    let response = op.response.clone().unwrap_or_else(|| "unknown".into());
    let path: String = op
        .segments
        .iter()
        .map(|s| match s {
            Segment::Literal { text } => text.clone(),
            Segment::Param { name } => {
                format!("${{codegen.pathSegment({})}}", naming.field_name(name))
            }
        })
        .collect();
    let mut fields = vec![
        format!("name: \"{}\"", op.hook_name),
        format!("method: \"{}\"", method(op.method)),
        format!("path: `{path}`"),
    ];
    if !op.query.is_empty() {
        let q: Vec<String> = op
            .query
            .iter()
            .map(|q| {
                let access = if key(&q.name) == q.name {
                    format!("params.{}", q.name)
                } else {
                    format!("params[\"{}\"]", q.name)
                };
                format!("[\"{}\", {access}]", q.name)
            })
            .collect();
        fields.push(format!("query: [{}]", q.join(", ")));
    }
    if let Some(b) = &op.body {
        if let Some(s) = shape_literal(&Type::Ref { name: b.clone() }, needs) {
            *uses_shapes = true;
            fields.push(format!("body: codegen.encode(body, {s}, shapes)"));
        } else {
            fields.push("body".into());
        }
    }
    let scopes: Vec<String> = op.scopes.iter().map(|s| format!("\"{s}\"")).collect();
    fields.push(format!("scopes: [{}]", scopes.join(", ")));
    if op.idempotent_override {
        fields.push("idempotent: true".into());
    }
    let mut out = format!(
        "\n{}export function {}({decl}): Promise<Response<{response}>> {{\n  return client.request(\n    {{\n",
        op_doc(op, ""),
        function_name(op)
    );
    for f in &fields {
        let _ = writeln!(out, "      {f},");
    }
    out.push_str("    },\n    options,\n");
    if let Some(s) = op
        .response
        .as_ref()
        .and_then(|r| shape_literal(&Type::Ref { name: r.clone() }, needs))
    {
        *uses_shapes = true;
        let _ = writeln!(out, "    {s},\n    shapes,");
    }
    out.push_str("  );\n}\n");
    out
}

fn render_profiles(surface: &context::Surface, runtime: &str) -> String {
    let mut models = BTreeSet::new();
    let mut params = BTreeSet::new();
    let mut body = String::new();
    for p in &surface.profiles {
        body.push_str(&profile_class(surface, p, &mut models, &mut params));
    }
    let mut out =
        format!("import {{ type CallOptions, Client, type Response }} from \"{runtime}\";\n");
    if !models.is_empty() {
        let list: Vec<String> = models.iter().map(|m| format!("type {m}")).collect();
        let _ = writeln!(
            out,
            "import {{ {} }} from \"./models.js\";",
            list.join(", ")
        );
    }
    out.push_str("import * as ops from \"./operations.js\";\n");
    if !params.is_empty() {
        let list: Vec<String> = params.iter().map(|m| format!("type {m}")).collect();
        let _ = writeln!(
            out,
            "import {{ {} }} from \"./operations.js\";",
            list.join(", ")
        );
    }
    out.push_str(&body);
    out
}

/// One profile's class and its handle classes.
fn profile_class(
    surface: &context::Surface,
    p: &context::Profile,
    models: &mut BTreeSet<String>,
    params: &mut BTreeSet<String>,
) -> String {
    let naming = TsNaming;
    let class = naming.type_name(&p.name);
    let handles: Vec<(&context::Handle, Vec<&Op>)> = surface
        .handles
        .iter()
        .map(|h| {
            let ops: Vec<&Op> = h
                .ops
                .iter()
                .filter(|o| o.profiles.contains(&p.name))
                .collect();
            (h, ops)
        })
        .filter(|(_, ops)| !ops.is_empty())
        .collect();
    let env_doc = if p.is_public {
        "`fromEnv()` reads `INORBIT_TOKEN`, or `INORBIT_KEY_ID`, `INORBIT_KEY_SECRET` and `INORBIT_SCOPES`".to_owned()
    } else {
        format!(
            "`fromEnv()` reads `INORBIT_{e}_TOKEN`, or `INORBIT_{e}_KEY_ID`, `INORBIT_{e}_KEY_SECRET` and `INORBIT_{e}_SCOPES`, and nothing else",
            e = p.env
        )
    };
    let mut out = format!(
        "\n/**\n * Profile `{}`: the operations its cut holds. {env_doc}.\n */\nexport class {class} {{\n  /** The profile's name. */\n  static readonly profile = \"{}\";\n",
        p.name, p.name
    );
    for (h, _) in &handles {
        let _ = writeln!(
            out,
            "  /** The `{}` operations. */\n  readonly {}: {class}{};",
            h.tag,
            naming.method_name(&h.tag),
            naming.type_name(&h.tag)
        );
    }
    out.push_str("\n  /** A profile on a client built for its credential. */\n");
    if handles.is_empty() {
        out.push_str("  constructor(readonly client: Client) {}\n");
    } else {
        out.push_str("  constructor(readonly client: Client) {\n");
        for (h, _) in &handles {
            let _ = writeln!(
                out,
                "    this.{} = new {class}{}(client);",
                naming.method_name(&h.tag),
                naming.type_name(&h.tag)
            );
        }
        out.push_str("  }\n");
    }
    let env_arg = if p.is_public {
        String::new()
    } else {
        format!("\"{}\"", p.env)
    };
    let _ = write!(
        out,
        "\n  /** The profile with its credential from the environment. */\n  static fromEnv(): {class} {{\n    return new {class}(Client.fromEnv({env_arg}));\n  }}\n"
    );
    for op in surface.flat.iter().filter(|o| o.profiles.contains(&p.name)) {
        out.push_str(&profile_method(op, models, params));
    }
    out.push_str("}\n");
    for (h, ops) in &handles {
        let _ = write!(
            out,
            "\n/** The `{}` operations profile `{}` may call. */\nexport class {class}{} {{\n  /** The handle on a profile's client. */\n  constructor(private readonly client: Client) {{}}\n",
            h.tag,
            p.name,
            naming.type_name(&h.tag)
        );
        for op in ops {
            out.push_str(&profile_method(op, models, params));
        }
        out.push_str("}\n");
    }
    out
}

fn profile_method(op: &Op, models: &mut BTreeSet<String>, params: &mut BTreeSet<String>) -> String {
    let naming = TsNaming;
    let (decl, call) = arguments(op);
    let decl = decl.trim_start_matches("client: Client, ").to_owned();
    let response = op.response.clone().unwrap_or_else(|| "unknown".into());
    if let Some(r) = &op.response {
        models.insert(r.clone());
    }
    if let Some(b) = &op.body {
        models.insert(b.clone());
    }
    if let Some(t) = &op.params_type {
        params.insert(t.clone());
    }
    format!(
        "\n{}  {}({decl}): Promise<Response<{response}>> {{\n    return ops.{}({call});\n  }}\n",
        op_doc(op, "  "),
        naming.method_name(&op.name),
        function_name(op)
    )
}

fn render_index(surface: &context::Surface, runtime: &str) -> String {
    let profiles: Vec<String> = surface
        .profiles
        .iter()
        .map(|p| format!("`{}`", p.name))
        .collect();
    let mut out = String::new();
    let _ = write!(
        out,
        "/**\n * The InOrbit API surface for {} (API {}).\n *\n * Generated by `iohr sdk generate`: the models and the operations these profiles may\n * call, on the `{runtime}` runtime. Regenerate it, never edit it; `iohr sdk check` says\n * when the API's cut has moved.\n *\n * @module\n */\n\nimport {{ codegen }} from \"{runtime}\";\n\n",
        profiles.join(", "),
        surface.api_version
    );
    out.push_str(
        "/** A surface generated for another runtime version fails to compile here: run `iohr sdk generate` again. */\nexport const SURFACE_VERSION: 1 = codegen.VERSION;\n\n",
    );
    out.push_str("export type * from \"./models.js\";\nexport type * from \"./operations.js\";\nexport * from \"./profiles.js\";\n");
    out
}
