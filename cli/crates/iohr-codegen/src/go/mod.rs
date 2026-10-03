//! The Go target: the models as structs in a `models` package, every operation as a
//! function in `internal/ops` (which only the surface's own packages can import), and
//! one package per profile whose client and handles hold only the operations its cut
//! holds, so a call the profile may not make does not build.
//!
//! Go needs import paths, so the surface is rendered for one: `--package` (the import
//! path of the output directory), which `iohr sdk generate` reads from the enclosing
//! `go.mod` when it is not given ([`package_for`]). Inside the runtime's own module
//! (`--in-package`) the one profile, `public`, is the root package itself.
//!
//! The output is gofmt-clean by construction: every struct field and constant stands
//! alone, so nothing needs column alignment, and imports are grouped and sorted.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use heck::ToSnakeCase as _;
use iohr_openapi::Api;

use crate::context::{self, Naming, Op, Segment};
use crate::files::Files;
use crate::ir::{self, Model, Shape, Type};
use crate::language::{Language, Options};
use crate::target::{RenderError, Target};

/// The Go target.
#[derive(Debug, Clone, Copy, Default)]
pub struct GoTarget;

/// The import path of the runtime's own public surface.
const PUBLIC_PACKAGE: &str = "github.com/inorbithr/sdk/go/public";

/// Words Go spells in capitals inside an identifier (`OrgID`, `BaseURL`), after the
/// list the Go project's own linters use.
const INITIALISMS: &[&str] = &[
    "ACL", "API", "ASCII", "CPU", "CSS", "DNS", "EOF", "GUID", "HTML", "HTTP", "HTTPS", "ID", "IP",
    "JSON", "OS", "QPS", "RAM", "RPC", "SLA", "SMTP", "SQL", "SSE", "SSH", "TCP", "TLS", "TTL",
    "UDP", "UI", "UID", "URI", "URL", "UTF8", "UUID", "VM", "XML",
];

/// How Go spells names: exported `UpperCamel` with initialisms in capitals for types,
/// methods and fields; `lowerCamel` for arguments.
struct GoNaming;

impl Naming for GoNaming {
    const RESERVED: &'static [&'static str] = &[
        "break",
        "case",
        "chan",
        "const",
        "continue",
        "default",
        "defer",
        "else",
        "fallthrough",
        "for",
        "func",
        "go",
        "goto",
        "if",
        "import",
        "interface",
        "map",
        "package",
        "range",
        "return",
        "select",
        "struct",
        "switch",
        "type",
        "var",
        // Names the generated functions use for their own arguments and imports.
        "body",
        "c",
        "codegen",
        "context",
        "ctx",
        "inorbit",
        "json",
        "models",
        "op",
        "ops",
        "params",
        "q",
        "strconv",
    ];

    fn type_name(&self, raw: &str) -> String {
        exported(raw)
    }

    fn method_name(&self, snake: &str) -> String {
        exported(snake)
    }

    fn field_name(&self, wire: &str) -> String {
        exported(wire)
    }
}

/// `org_id` as `OrgID`, `base_url` as `BaseURL`.
fn exported(raw: &str) -> String {
    let words = words(raw);
    let mut out: String = words.iter().map(|w| capital(w)).collect();
    if out.is_empty() {
        out.push('X');
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 'X');
    }
    out
}

/// `org_id` as `orgID`, an argument's name; a reserved word gains a trailing `_`.
fn unexported(raw: &str) -> String {
    let words = words(raw);
    let mut out = String::new();
    for (i, w) in words.iter().enumerate() {
        if i == 0 {
            out.push_str(&w.to_ascii_lowercase());
        } else {
            out.push_str(&capital(w));
        }
    }
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 'v');
    }
    GoNaming.escape(out)
}

fn words(raw: &str) -> Vec<String> {
    raw.to_snake_case()
        .split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            w.chars()
                .filter(char::is_ascii_alphanumeric)
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

fn capital(word: &str) -> String {
    let upper = word.to_ascii_uppercase();
    if INITIALISMS.contains(&upper.as_str()) {
        return upper;
    }
    let mut c = word.chars();
    c.next().map_or_else(String::new, |f| {
        f.to_ascii_uppercase().to_string() + &c.as_str().to_ascii_lowercase()
    })
}

/// A Go package name for a profile or a path segment: lower case letters and digits.
fn package_name(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 'p');
    }
    if GoNaming::RESERVED.contains(&out.as_str()) {
        out.push_str("api");
    }
    out
}

/// The import path of a directory, read from the nearest `go.mod` above it: the
/// module's path joined with the directory's place inside the module. `None` when no
/// `go.mod` encloses it, or the module line cannot be read.
#[must_use]
pub fn package_for(out: &Path) -> Option<String> {
    let abs = if out.is_absolute() {
        out.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(out)
    };
    let mut rel: Vec<String> = Vec::new();
    let mut dir = abs.as_path();
    loop {
        let gomod = dir.join("go.mod");
        if let Ok(text) = std::fs::read_to_string(&gomod) {
            let module = text.lines().find_map(|l| {
                l.trim()
                    .strip_prefix("module ")
                    .map(|m| m.trim().trim_matches('"').to_owned())
            })?;
            rel.reverse();
            let mut path = module;
            for part in rel {
                path.push('/');
                path.push_str(&part);
            }
            return Some(path);
        }
        let name = dir.file_name()?.to_str()?.to_owned();
        if name != "." {
            rel.push(name);
        }
        dir = dir.parent()?;
    }
}

/// The text of a Go comment: one line, no line breaks.
fn comment_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A doc comment for `name`, starting with it as Go documentation does.
fn doc_for(name: &str, text: &str, indent: &str) -> String {
    let text = comment_text(text);
    if text.is_empty() {
        String::new()
    } else {
        format!("{indent}// {name}: {text}\n")
    }
}

/// The surface's paths and package names.
struct Layout {
    /// The runtime's import path.
    runtime: String,
    /// The import path of the surface's root.
    root: String,
    /// `true` inside the runtime's own module: the one profile is the root package.
    in_package: bool,
}

impl Layout {
    fn models(&self) -> String {
        format!("{}/models", self.root)
    }

    fn ops(&self) -> String {
        format!("{}/internal/ops", self.root)
    }

    fn codegen(&self) -> String {
        format!("{}/codegen", self.runtime)
    }
}

/// The imports of one file, grouped: the standard library, then the rest.
#[derive(Default)]
struct Imports {
    std: BTreeSet<&'static str>,
    other: BTreeMap<String, Option<&'static str>>,
}

impl Imports {
    fn std(&mut self, path: &'static str) {
        self.std.insert(path);
    }

    fn other(&mut self, path: &str, alias: Option<&'static str>) {
        self.other.insert(path.to_owned(), alias);
    }

    fn render(&self) -> String {
        if self.std.is_empty() && self.other.is_empty() {
            return String::new();
        }
        let mut out = String::from("import (\n");
        for p in &self.std {
            let _ = writeln!(out, "\t\"{p}\"");
        }
        if !self.std.is_empty() && !self.other.is_empty() {
            out.push('\n');
        }
        for (p, alias) in &self.other {
            match alias {
                Some(a) => {
                    let _ = writeln!(out, "\t{a} \"{p}\"");
                }
                None => {
                    let _ = writeln!(out, "\t\"{p}\"");
                }
            }
        }
        out.push_str(")\n\n");
        out
    }
}

impl Target for GoTarget {
    const LANG: Language = Language::Go;

    fn render(&self, api: &Api, options: &Options) -> Result<Files, RenderError> {
        let root = match (&options.package, options.in_package) {
            (Some(p), _) => p.trim_end_matches('/').to_owned(),
            (None, true) => PUBLIC_PACKAGE.to_owned(),
            (None, false) => {
                return Err(RenderError::Models(
                    "a Go surface needs the import path of its directory: pass --package (such as example.com/app/iohr), or generate inside a Go module so it is read from go.mod".into(),
                ));
            }
        };
        let layout = Layout {
            runtime: options.runtime_for(Self::LANG),
            root,
            in_package: options.in_package,
        };
        let surface = context::surface(api);
        if layout.in_package && surface.profiles.len() != 1 {
            return Err(RenderError::Models(
                "the runtime's own surface is one profile".into(),
            ));
        }
        let mut seen = BTreeMap::new();
        for p in &surface.profiles {
            if let Some(other) = seen.insert(package_name(&p.name), p.name.clone()) {
                return Err(RenderError::Models(format!(
                    "the profiles {other} and {} would both be the Go package {}; rename one",
                    p.name,
                    package_name(&p.name)
                )));
            }
        }
        let models = context::models(api);
        let nilable = nilable_models(&models);
        let header = go_header(&surface.header);
        let mut files = Files::new();
        for note in &surface.notes {
            files.note(note.clone());
        }
        let ops = all_ops(&surface);
        files.insert(
            "models/models.go",
            format!(
                "{header}{}",
                render_models(&models, &ops, &nilable, &layout)
            ),
        );
        files.insert(
            "internal/ops/ops.go",
            format!("{header}{}", render_ops(&ops, &layout)),
        );
        for p in &surface.profiles {
            let (dir, name) = if layout.in_package {
                (
                    String::new(),
                    package_name(layout.root.rsplit('/').next().unwrap_or("public")),
                )
            } else {
                (format!("{}/", package_name(&p.name)), package_name(&p.name))
            };
            let text = render_profile(&surface, p, &name, &layout, &models)?;
            files.insert(format!("{dir}client.go"), format!("{header}{text}"));
        }
        Ok(files)
    }
}

/// The line Go tools recognise as generated code (`^// Code generated .* DO NOT EDIT\.$`).
fn go_header(header: &str) -> String {
    let rest = header
        .strip_prefix("Generated by ")
        .unwrap_or(header)
        .trim_end_matches("; do not edit.");
    format!("// Code generated by {rest}; DO NOT EDIT.\n\n")
}

/// Every rendered operation, in `METHOD /path` order.
fn all_ops(surface: &context::Surface) -> Vec<Op> {
    let mut ops: Vec<Op> = surface
        .handles
        .iter()
        .flat_map(|h| h.ops.iter().cloned())
        .chain(surface.flat.iter().cloned())
        .collect();
    ops.sort_by(|a, b| a.line.cmp(&b.line));
    ops
}

/// The models whose Go type is already nil-able (a list, a map, raw JSON), so an
/// optional field of that type needs no pointer.
fn nilable_models(models: &[Model]) -> BTreeSet<String> {
    models
        .iter()
        .filter(|m| match &m.shape {
            Shape::Union { .. } => true,
            Shape::Alias { ty } => matches!(ty, Type::Array { .. } | Type::Map { .. } | Type::Any),
            _ => false,
        })
        .map(|m| m.name.clone())
        .collect()
}

/// What a type expression needs imported.
#[derive(Default)]
struct Uses {
    runtime: bool,
    json: bool,
    models: bool,
}

/// A type expression; `q` qualifies a model (`models.` outside the models package).
fn go_type(ty: &Type, q: &str, uses: &mut Uses) -> String {
    match ty {
        Type::Ref { name } if ir::RUNTIME_TYPES.contains(&name.as_str()) => {
            uses.runtime = true;
            format!("inorbit.{name}")
        }
        Type::Ref { name } => {
            if !q.is_empty() {
                uses.models = true;
            }
            format!("{q}{}", GoNaming.type_name(name))
        }
        Type::Int64 => {
            uses.runtime = true;
            "inorbit.Int64".into()
        }
        Type::Integer { bits: 32 } => "int32".into(),
        Type::Integer { .. } => "int64".into(),
        Type::Number => "float64".into(),
        Type::String | Type::Enum { .. } => "string".into(),
        Type::Bool => "bool".into(),
        Type::Array { item } => format!("[]{}", go_type(item, q, uses)),
        Type::Map { value } => format!("map[string]{}", go_type(value, q, uses)),
        _ => {
            uses.json = true;
            "json.RawMessage".into()
        }
    }
}

/// Whether a value of `ty` can already be nil.
fn is_nilable(ty: &Type, nilable: &BTreeSet<String>) -> bool {
    match ty {
        Type::Ref { name } => nilable.contains(name),
        Type::Int64
        | Type::Integer { .. }
        | Type::Number
        | Type::String
        | Type::Enum { .. }
        | Type::Bool => false,
        _ => true,
    }
}

/// Field names unique within one struct: a second wire name that spells the same Go
/// name gains its position.
fn field_names<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut used = BTreeSet::new();
    names
        .enumerate()
        .map(|(i, n)| {
            let mut name = GoNaming.field_name(n);
            if !used.insert(name.clone()) {
                name = format!("{name}{i}");
                used.insert(name.clone());
            }
            name
        })
        .collect()
}

fn render_models(
    models: &[Model],
    ops: &[Op],
    nilable: &BTreeSet<String>,
    layout: &Layout,
) -> String {
    let mut uses = Uses::default();
    let mut body = String::new();
    for m in models.iter().filter(|m| !m.runtime) {
        body.push('\n');
        body.push_str(&model_decl(m, nilable, &mut uses));
    }
    for op in ops {
        if let Some(t) = &op.params_type {
            body.push('\n');
            body.push_str(&params_struct(t, op, &mut uses));
        }
    }
    let mut imports = Imports::default();
    if uses.json {
        imports.std("encoding/json");
    }
    if uses.runtime {
        imports.other(&layout.runtime, Some("inorbit"));
    }
    format!(
        "// Package models holds the models of the InOrbit API surface: what the operations take and answer.\npackage models\n\n{}{}",
        imports.render(),
        body.trim_start_matches('\n')
    )
}

/// A model's doc comment: its description, or `fallback` when it has none.
fn model_doc(name: &str, m: &Model, fallback: &str) -> String {
    if m.doc.is_empty() {
        format!("// {name} {fallback}\n")
    } else {
        doc_for(name, &m.doc, "")
    }
}

/// One model's declaration.
fn model_decl(m: &Model, nilable: &BTreeSet<String>, uses: &mut Uses) -> String {
    let name = GoNaming.type_name(&m.name);
    let mut out = String::new();
    match &m.shape {
        Shape::Object { fields } => {
            out.push_str(&model_doc(
                &name,
                m,
                &format!("is the API's {} model.", m.name),
            ));
            if fields.is_empty() {
                let _ = writeln!(out, "type {name} struct{{}}");
                return out;
            }
            let _ = writeln!(out, "type {name} struct {{");
            let names = field_names(fields.iter().map(|f| f.name.as_str()));
            for (i, (f, go_name)) in fields.iter().zip(&names).enumerate() {
                if i > 0 {
                    out.push('\n');
                }
                let mut ty = go_type(&f.ty, "", uses);
                if (!f.required || f.nullable) && !is_nilable(&f.ty, nilable) {
                    ty.insert(0, '*');
                }
                let omit = if f.required { "" } else { ",omitempty" };
                let doc = comment_text(&f.doc);
                if !doc.is_empty() {
                    let _ = writeln!(out, "\t// {doc}");
                }
                let _ = writeln!(out, "\t{go_name} {ty} `json:\"{}{omit}\"`", f.name);
            }
            out.push_str("}\n");
        }
        Shape::Enum { values } => {
            out.push_str(&model_doc(&name, m, "is one of a fixed set of values."));
            let _ = writeln!(out, "type {name} string");
            let mut consts = BTreeSet::new();
            for v in values {
                let mut c = format!("{name}{}", exported(v));
                if !consts.insert(c.clone()) {
                    c = format!("{c}{}", consts.len());
                    consts.insert(c.clone());
                }
                let _ = write!(out, "\n// {c} is {name} {v:?}.\nconst {c} {name} = {v:?}\n");
            }
        }
        Shape::Union { variants, .. } => {
            let names: Vec<String> = variants
                .iter()
                .map(|v| go_type(v, "", &mut Uses::default()))
                .collect();
            let _ = writeln!(
                out,
                "// {name} is one of {}, as the API sent it; unmarshal it into the one its fields name.",
                names.join(", ")
            );
            uses.json = true;
            let _ = writeln!(out, "type {name} = json.RawMessage");
        }
        Shape::Alias { ty } => {
            out.push_str(&model_doc(&name, m, "is another name for its type."));
            let _ = writeln!(out, "type {name} = {}", go_type(ty, "", uses));
        }
    }
    out
}

/// The query parameters of one operation, as a struct: a required one is a value, an
/// optional one a pointer (or a nil-able list).
fn params_struct(type_name: &str, op: &Op, uses: &mut Uses) -> String {
    let mut out = format!(
        "// {type_name} holds the query parameters of {}.\ntype {type_name} struct {{\n",
        op.line
    );
    let names = field_names(op.query.iter().map(|q| q.name.as_str()));
    for (i, (q, go_name)) in op.query.iter().zip(&names).enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let mut ty = go_type(&q.ty, "", uses);
        if !q.required && !matches!(q.ty, Type::Array { .. } | Type::Map { .. }) {
            ty.insert(0, '*');
        }
        let doc = comment_text(&q.doc);
        let doc = if doc.is_empty() {
            format!("{} is the {} parameter.", go_name, q.name)
        } else {
            doc
        };
        let _ = writeln!(out, "\t// {doc}\n\t{go_name} {ty}");
    }
    out.push_str("}\n");
    out
}

/// The function name of an operation in `internal/ops`: its marker, which is unique.
fn function_name(op: &Op) -> String {
    exported(&op.marker)
}

fn scopes_note(op: &Op) -> String {
    match op.scopes.len() {
        0 => String::new(),
        1 => format!("; needs scope {}", op.scopes[0]),
        _ => format!("; needs scopes {}", op.scopes.join(", ")),
    }
}

/// A method's doc comment.
fn op_doc(name: &str, op: &Op, indent: &str) -> String {
    let mut text = format!("{name} calls {}{}.", op.line, scopes_note(op));
    let extra = comment_text(&op.doc);
    if !extra.is_empty() {
        let _ = write!(text, " {extra}");
    }
    format!("{indent}// {text}\n")
}

/// The arguments of an operation after `ctx`: declarations and the names to pass on.
fn arguments(op: &Op, uses: &mut Uses) -> (Vec<String>, Vec<String>) {
    let mut decl = Vec::new();
    let mut call = Vec::new();
    for p in &op.path_params {
        let name = unexported(&p.name);
        decl.push(format!("{name} string"));
        call.push(name);
    }
    if let Some(t) = &op.params_type {
        uses.models = true;
        if op.query.iter().any(|q| q.required) {
            decl.push(format!("params models.{t}"));
        } else {
            decl.push(format!("params *models.{t}"));
        }
        call.push("params".into());
    }
    if let Some(b) = &op.body {
        uses.models = true;
        decl.push(format!("body models.{}", GoNaming.type_name(b)));
        call.push("body".into());
    }
    (decl, call)
}

fn response_type(op: &Op, uses: &mut Uses) -> String {
    if let Some(r) = &op.response {
        uses.models = true;
        format!("models.{}", GoNaming.type_name(r))
    } else {
        uses.json = true;
        "json.RawMessage".into()
    }
}

/// A query value as a string expression, and whether it needs strconv.
fn query_value(ty: &Type, expr: &str, strconv: &mut bool) -> String {
    match ty {
        Type::String | Type::Enum { .. } => expr.to_owned(),
        Type::Int64 => format!("{expr}.String()"),
        Type::Integer { .. } => {
            *strconv = true;
            format!("strconv.FormatInt(int64({expr}), 10)")
        }
        Type::Number => {
            *strconv = true;
            format!("strconv.FormatFloat({expr}, 'g', -1, 64)")
        }
        Type::Bool => {
            *strconv = true;
            format!("strconv.FormatBool({expr})")
        }
        _ => format!("string({expr})"),
    }
}

/// What the operations file needs imported, beyond `context` and the runtime.
#[derive(Default)]
struct OpsUses {
    types: Uses,
    strconv: bool,
    codegen: bool,
}

fn render_ops(ops: &[Op], layout: &Layout) -> String {
    let mut uses = OpsUses::default();
    let mut body = String::new();
    for op in ops {
        body.push('\n');
        body.push_str(&operation_function(op, &mut uses));
    }
    let mut imports = Imports::default();
    imports.std("context");
    if uses.types.json {
        imports.std("encoding/json");
    }
    if uses.strconv {
        imports.std("strconv");
    }
    imports.other(&layout.runtime, Some("inorbit"));
    if uses.codegen {
        imports.other(&layout.codegen(), None);
    }
    if uses.types.models {
        imports.other(&layout.models(), None);
    }
    format!(
        "// Package ops holds every operation of the surface as a function over the runtime's one request path; only the surface's own packages can import it.\npackage ops\n\n{}{}",
        imports.render(),
        body.trim_start_matches('\n')
    )
}

/// One operation as a function over the runtime's request path.
fn operation_function(op: &Op, uses: &mut OpsUses) -> String {
    let name = function_name(op);
    let (decl, _) = arguments(op, &mut uses.types);
    let response = response_type(op, &mut uses.types);
    let mut sig = vec![
        "ctx context.Context".to_owned(),
        "c *inorbit.Client".to_owned(),
    ];
    sig.extend(decl);
    let mut out = op_doc(&name, op, "");
    let _ = writeln!(
        out,
        "func {name}({}) (*inorbit.Response[{response}], error) {{",
        sig.join(", ")
    );
    let path: Vec<String> = op
        .segments
        .iter()
        .map(|s| match s {
            Segment::Literal { text } => format!("{text:?}"),
            Segment::Param { name } => {
                uses.codegen = true;
                format!("codegen.PathSegment({})", unexported(name))
            }
        })
        .collect();
    let scopes: Vec<String> = op.scopes.iter().map(|s| format!("{s:?}")).collect();
    let mut fields = vec![
        format!("Name: {:?}", op.hook_name),
        format!("Method: {:?}", op.method.as_str()),
        format!("Path: {}", path.join(" + ")),
    ];
    if !scopes.is_empty() {
        fields.push(format!("Scopes: []string{{{}}}", scopes.join(", ")));
    }
    // One line: gofmt aligns the keys of a literal spread over several lines.
    let _ = writeln!(out, "\top := inorbit.Operation{{{}}}", fields.join(", "));
    if op.idempotent_override {
        out.push_str("\top.Idempotent = true\n");
    }
    if op.body.is_some() {
        out.push_str("\top.Body = body\n");
    }
    if op.params_type.is_some() {
        uses.codegen = true;
        out.push_str(&query_block(op, &mut uses.strconv));
    }
    let _ = writeln!(out, "\treturn inorbit.Call[{response}](ctx, c, op)\n}}");
    out
}

/// The statements that copy an operation's query parameters onto `op.Query`.
fn query_block(op: &Op, strconv: &mut bool) -> String {
    let mut out = String::new();
    let pointer = !op.query.iter().any(|q| q.required);
    let indent = if pointer { "\t\t" } else { "\t" };
    if pointer {
        out.push_str("\tif params != nil {\n");
    }
    let _ = writeln!(out, "{indent}q := codegen.Query()");
    let names = field_names(op.query.iter().map(|q| q.name.as_str()));
    for (q, field) in op.query.iter().zip(&names) {
        let access = format!("params.{field}");
        match &q.ty {
            Type::Array { item } => {
                let v = query_value(item, "v", strconv);
                let _ = writeln!(
                    out,
                    "{indent}for _, v := range {access} {{\n{indent}\tq.Add({:?}, {v})\n{indent}}}",
                    q.name
                );
            }
            ty if !q.required => {
                let v = query_value(ty, &format!("*{access}"), strconv);
                let _ = writeln!(
                    out,
                    "{indent}if {access} != nil {{\n{indent}\tq.Set({:?}, {v})\n{indent}}}",
                    q.name
                );
            }
            ty => {
                let v = query_value(ty, &access, strconv);
                let _ = writeln!(out, "{indent}q.Set({:?}, {v})", q.name);
            }
        }
    }
    let _ = writeln!(out, "{indent}op.Query = q");
    if pointer {
        out.push_str("\t}\n");
    }
    out
}

/// The accessor method and the type of each tag's handle, by tag, checked against the
/// flat methods: one profile's client cannot have two members of one name.
fn member_names(
    handles: &[(&context::Handle, Vec<&Op>)],
    flat: &[&Op],
) -> Result<BTreeMap<String, (String, String)>, RenderError> {
    let mut taken: BTreeSet<String> = ["Runtime".to_owned()].into();
    let mut handle_types = BTreeMap::new();
    for (h, _) in handles {
        let accessor = GoNaming.type_name(&h.tag);
        let mut ty = accessor.clone();
        if ty == "Client" || ty == "Profile" {
            ty.push_str("Ops");
        }
        if !taken.insert(accessor.clone()) {
            return Err(RenderError::Models(format!(
                "the tag {} would be the method {accessor} twice",
                h.tag
            )));
        }
        handle_types.insert(h.tag.clone(), (accessor, ty));
    }
    for op in flat {
        let m = GoNaming.method_name(&op.name);
        if !taken.insert(m.clone()) {
            return Err(RenderError::Models(format!(
                "the operation {} would be the method {m}, which another name already is",
                op.line
            )));
        }
    }
    Ok(handle_types)
}

/// One profile's package: its client, the handles, and the methods its cut holds.
fn render_profile(
    surface: &context::Surface,
    p: &context::Profile,
    pkg: &str,
    layout: &Layout,
    models: &[Model],
) -> Result<String, RenderError> {
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
    let flat: Vec<&Op> = surface
        .flat
        .iter()
        .filter(|o| o.profiles.contains(&p.name))
        .collect();
    let handle_types = member_names(&handles, &flat)?;
    let mut uses = Uses::default();
    // Whether an iterator over a paged list is rendered, which imports iter.
    let mut paged = false;
    let mut body = String::new();
    let env_doc = if p.is_public {
        "FromEnv reads INORBIT_TOKEN, or INORBIT_KEY_ID, INORBIT_KEY_SECRET and INORBIT_SCOPES"
            .to_owned()
    } else {
        format!(
            "FromEnv reads INORBIT_{e}_TOKEN, or INORBIT_{e}_KEY_ID, INORBIT_{e}_KEY_SECRET and INORBIT_{e}_SCOPES, and nothing else",
            e = p.env
        )
    };
    let env_arg = if p.is_public {
        String::new()
    } else {
        p.env.clone()
    };
    let _ = write!(
        body,
        "// The surface was generated for the runtime's contract 2: a runtime with another contract fails to build here, so run iohr sdk generate again.\nconst _ = codegen.V2\n\n// Profile is the profile's name.\nconst Profile = {name:?}\n\n// Client calls the operations profile {name} may call.\ntype Client struct {{\n\tc *inorbit.Client\n}}\n\n// New returns the profile's surface on c, a client built for its credential.\nfunc New(c *inorbit.Client) *Client {{\n\treturn &Client{{c: c}}\n}}\n\n// FromEnv returns the profile's surface with its credential from the environment; opts apply after it.\nfunc FromEnv(opts ...inorbit.Option) (*Client, error) {{\n\tc, err := inorbit.FromEnv({env_arg:?}, opts...)\n\tif err != nil {{\n\t\treturn nil, err\n\t}}\n\treturn New(c), nil\n}}\n\n// Runtime returns the client underneath, for a raw call with Send.\nfunc (p *Client) Runtime() *inorbit.Client {{\n\treturn p.c\n}}\n",
        name = p.name
    );
    for op in &flat {
        body.push('\n');
        body.push_str(&method(op, "p *Client", "p.c", &mut uses));
        if let Some(paging) = paging(op, models) {
            paged = true;
            body.push('\n');
            body.push_str(&iterator(op, &paging, "p *Client", "p.c", &mut uses));
        }
    }
    for (h, ops) in &handles {
        let (accessor, ty) = &handle_types[&h.tag];
        let _ = write!(
            body,
            "\n// {accessor} returns the {tag} operations the profile may call.\nfunc (p *Client) {accessor}() {ty} {{\n\treturn {ty}{{c: p.c}}\n}}\n\n// {ty} holds the {tag} operations profile {name} may call.\ntype {ty} struct {{\n\tc *inorbit.Client\n}}\n",
            tag = h.tag,
            name = p.name
        );
        for op in ops {
            body.push('\n');
            body.push_str(&method(op, &format!("h {ty}"), "h.c", &mut uses));
            if let Some(paging) = paging(op, models) {
                paged = true;
                body.push('\n');
                body.push_str(&iterator(op, &paging, &format!("h {ty}"), "h.c", &mut uses));
            }
        }
    }
    let mut imports = Imports::default();
    if !flat.is_empty() || !handles.is_empty() {
        imports.std("context");
    }
    if uses.json {
        imports.std("encoding/json");
    }
    if paged {
        imports.std("iter");
    }
    imports.other(&layout.runtime, Some("inorbit"));
    imports.other(&layout.codegen(), None);
    if !flat.is_empty() || !handles.is_empty() {
        imports.other(&layout.ops(), None);
    }
    if uses.models {
        imports.other(&layout.models(), None);
    }
    Ok(format!(
        "// Package {pkg} is the InOrbit API surface for profile {}: only the operations its cut holds, so a call it may not make does not build. {env_doc}.\npackage {pkg}\n\n{}{}",
        p.name,
        imports.render(),
        body
    ))
}

/// A profile or handle method that forwards to the operation's function.
fn method(op: &Op, receiver: &str, client: &str, uses: &mut Uses) -> String {
    let name = GoNaming.method_name(&op.name);
    let (decl, call) = arguments(op, uses);
    let response = response_type(op, uses);
    let mut sig = vec!["ctx context.Context".to_owned()];
    sig.extend(decl);
    let mut args = vec!["ctx".to_owned(), client.to_owned()];
    args.extend(call);
    format!(
        "{}func ({receiver}) {name}({}) (*inorbit.Response[{response}], error) {{\n\treturn ops.{}({})\n}}\n",
        op_doc(&name, op, ""),
        sig.join(", "),
        function_name(op),
        args.join(", ")
    )
}

/// How a paged list operation pages: the query field that carries the token, and the
/// answer's list and next-token fields.
struct Paging {
    /// The Go type of one item.
    item: String,
    /// The answer's list field.
    list: String,
    /// The answer's next-token field, and whether it is a pointer.
    next: String,
    next_pointer: bool,
    /// The parameters' token field, and whether it is a pointer.
    token: String,
    token_pointer: bool,
}

/// The paging of `op`, when it pages: a `page_token` query parameter, and an answer with
/// a `next_page_token` string and exactly one list.
fn paging(op: &Op, models: &[Model]) -> Option<Paging> {
    const TOKEN: [&str; 2] = ["page_token", "pageToken"];
    const NEXT: [&str; 2] = ["next_page_token", "nextPageToken"];
    let token = op
        .query
        .iter()
        .position(|q| TOKEN.contains(&q.name.as_str()) && q.ty == Type::String)?;
    let response = op.response.as_deref()?;
    let Some(Model {
        shape: Shape::Object { fields },
        ..
    }) = models.iter().find(|m| m.name == response)
    else {
        return None;
    };
    let next = fields
        .iter()
        .position(|f| NEXT.contains(&f.name.as_str()) && f.ty == Type::String)?;
    let mut lists = fields
        .iter()
        .enumerate()
        .filter(|(_, f)| matches!(f.ty, Type::Array { .. }));
    let (list, list_field) = lists.next()?;
    if lists.next().is_some() {
        return None;
    }
    let Type::Array { item } = &list_field.ty else {
        return None;
    };
    let names = field_names(fields.iter().map(|f| f.name.as_str()));
    let query_names = field_names(op.query.iter().map(|q| q.name.as_str()));
    let next_field = &fields[next];
    Some(Paging {
        item: go_type(item, "models.", &mut Uses::default()),
        list: names[list].clone(),
        next: names[next].clone(),
        next_pointer: !next_field.required || next_field.nullable,
        token: query_names[token].clone(),
        token_pointer: !op.query[token].required,
    })
}

/// `All<Op>`: every item of a paged list, page after page, as a range-over-func iterator.
fn iterator(op: &Op, paging: &Paging, receiver: &str, client: &str, uses: &mut Uses) -> String {
    uses.models = true;
    let method = GoNaming.method_name(&op.name);
    let name = format!("All{method}");
    let (decl, call) = arguments(op, uses);
    let mut sig = vec!["ctx context.Context".to_owned()];
    sig.extend(decl);
    let params_type = op.params_type.as_deref().unwrap_or_default();
    let by_value = op.query.iter().any(|q| q.required);
    let copy = if by_value {
        "\t\tp := params\n".to_owned()
    } else {
        format!(
            "\t\tp := models.{params_type}{{}}\n\t\tif params != nil {{\n\t\t\tp = *params\n\t\t}}\n"
        )
    };
    let set_token = if paging.token_pointer {
        format!(
            "\t\tif token != \"\" {{\n\t\t\tp.{} = &token\n\t\t}}\n",
            paging.token
        )
    } else {
        format!("\t\tp.{} = token\n", paging.token)
    };
    let args: Vec<String> = ["ctx".to_owned(), client.to_owned()]
        .into_iter()
        .chain(call.into_iter().map(|a| match a.as_str() {
            "params" if by_value => "p".to_owned(),
            "params" => "&p".to_owned(),
            other => other.to_owned(),
        }))
        .collect();
    let next = if paging.next_pointer {
        format!(
            "\t\tnext := \"\"\n\t\tif r.Value.{n} != nil {{\n\t\t\tnext = *r.Value.{n}\n\t\t}}\n\t\treturn r.Value.{l}, next, nil\n",
            n = paging.next,
            l = paging.list
        )
    } else {
        format!(
            "\t\treturn r.Value.{}, r.Value.{}, nil\n",
            paging.list, paging.next
        )
    };
    format!(
        "// {name} iterates over every item {method} returns, page after page, following {next_field} until the last page. It stops at the first error, which it yields, and when the loop breaks or ctx is done.\nfunc ({receiver}) {name}({}) iter.Seq2[{item}, error] {{\n\treturn codegen.Pages(ctx, func(ctx context.Context, token string) ([]{item}, string, error) {{\n{copy}{set_token}\t\tr, err := ops.{}({})\n\t\tif err != nil {{\n\t\t\treturn nil, \"\", err\n\t\t}}\n{next}\t}})\n}}\n",
        sig.join(", "),
        function_name(op),
        args.join(", "),
        next_field = paging.next,
        item = paging.item,
    )
}

#[cfg(test)]
mod tests {
    use super::{exported, package_name, unexported};

    #[test]
    fn names_keep_initialisms_in_capitals() {
        assert_eq!(exported("org_id"), "OrgID");
        assert_eq!(exported("base_url"), "BaseURL");
        assert_eq!(exported("list_digests"), "ListDigests");
        assert_eq!(unexported("org_id"), "orgID");
        assert_eq!(unexported("id"), "id");
        assert_eq!(unexported("type"), "type_");
        assert_eq!(package_name("acme-ci"), "acmeci");
        assert_eq!(package_name("7up"), "p7up");
    }
}
