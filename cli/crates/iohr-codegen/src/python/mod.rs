//! The Python target: models as pydantic classes, each operation as a builder of the
//! runtime's `Operation`, and one class per profile, in a blocking and an `asyncio` form,
//! whose handles hold only the operations its cut holds, so a call the profile may not
//! make is an error under pyright and mypy.
//!
//! 64-bit integers are the runtime's `Int64`, an `int` that travels as a decimal string;
//! pydantic reads both forms and writes the string. A field a model's message may leave
//! out is `None`.

mod example;
pub(crate) use example::example;

use std::collections::BTreeSet;
use std::fmt::Write as _;

use heck::ToSnakeCase as _;
use iohr_openapi::Api;

use crate::context::{self, Naming, Op, Segment};
use crate::files::Files;
use crate::ir::{self, Field, Model, Shape, Type};
use crate::language::{Language, Options};
use crate::target::{RenderError, Target};

/// The Python target.
#[derive(Debug, Clone, Copy, Default)]
pub struct PythonTarget;

/// Python's keywords, and the names a generated method's own arguments use.
const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

/// Names a model field may not take: pydantic's own attributes on every model.
const MODEL_ATTRIBUTES: &[&str] = &[
    "construct",
    "copy",
    "dict",
    "from_orm",
    "json",
    "parse_file",
    "parse_obj",
    "parse_raw",
    "schema",
    "schema_json",
    "update_forward_refs",
    "validate",
];

/// How Python spells names: `snake_case` methods and arguments, `UpperCamel` types, a
/// keyword (or one of a method's own arguments) suffixed with `_`.
struct PyNaming;

impl Naming for PyNaming {
    const RESERVED: &'static [&'static str] = &[
        "False", "None", "True", "and", "as", "assert", "async", "await", "body", "break", "class",
        "cls", "continue", "def", "del", "elif", "else", "except", "finally", "for", "from",
        "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise",
        "return", "self", "timeout", "try", "while", "with", "yield",
    ];
}

impl Target for PythonTarget {
    const LANG: Language = Language::Python;

    fn render(&self, api: &Api, options: &Options) -> Result<Files, RenderError> {
        // Inside the runtime's own package the surface sits in `_generated/`, one level
        // down; anywhere else it imports the runtime by its name.
        let runtime = if options.in_package && options.runtime.is_none() {
            "..runtime".to_owned()
        } else {
            format!("{}.runtime", options.runtime_for(Self::LANG))
        };
        let surface = context::surface(api);
        let models = context::models(api);
        let mut files = Files::new();
        for note in &surface.notes {
            files.note(note.clone());
        }
        let header = format!("# {}\n", surface.header);
        files.insert(
            "models.py",
            format!("{header}{}", render_models(&models, &runtime)),
        );
        files.insert(
            "operations.py",
            format!("{header}{}", render_operations(&surface, &runtime)),
        );
        files.insert(
            "profiles.py",
            format!("{header}{}", render_profiles(&surface, &models, &runtime)),
        );
        files.insert(
            "__init__.py",
            format!(
                "{header}{}",
                render_init(
                    &surface,
                    &models,
                    &runtime,
                    &options.runtime_for(Self::LANG)
                )
            ),
        );
        files.insert("py.typed", String::new());
        Ok(files)
    }
}

/// One `from <module> import <names>` statement.
struct Import {
    module: String,
    names: Vec<String>,
}

impl Import {
    fn new(module: &str, names: impl IntoIterator<Item = String>) -> Self {
        Self {
            module: module.to_owned(),
            names: names.into_iter().collect(),
        }
    }
}

/// isort's order for imported names: constants, then classes, then everything else,
/// each case-insensitively.
fn name_key(name: &str) -> (u8, String) {
    let bare = name.split(" as ").next().unwrap_or(name);
    let class = if bare.len() > 1 && bare.chars().all(|c| !c.is_ascii_lowercase()) {
        0
    } else if bare.starts_with(|c: char| c.is_ascii_uppercase()) {
        1
    } else {
        2
    };
    (class, bare.to_lowercase())
}

/// isort's order for relative modules: the furthest first (`..runtime`, `.`, `.models`).
fn module_key(module: &str) -> (std::cmp::Reverse<usize>, String) {
    let dots = module.chars().take_while(|c| *c == '.').count();
    (std::cmp::Reverse(dots), module.to_owned())
}

/// The import section of a module: `__future__`, then the standard library, then other
/// packages, then the surface's own modules, each sorted as ruff's isort sorts them.
/// A statement with two names or more lists them one per line with a trailing comma, so
/// it reads the same at any line length.
fn imports(stdlib: Vec<Import>, packages: Vec<Import>) -> String {
    let (mut local, mut third): (Vec<Import>, Vec<Import>) = packages
        .into_iter()
        .partition(|i| i.module.starts_with('.'));
    let mut stdlib = stdlib;
    let mut out = String::from("from __future__ import annotations\n");
    for block in [&mut stdlib, &mut third, &mut local] {
        block.retain(|i| !i.names.is_empty());
        if block.is_empty() {
            continue;
        }
        block.sort_by_key(|i| module_key(&i.module));
        out.push('\n');
        for i in block.iter_mut() {
            i.names.sort_by_key(|n| name_key(n));
            i.names.dedup();
            if let [one] = i.names.as_slice() {
                let _ = writeln!(out, "from {} import {one}", i.module);
            } else {
                let _ = writeln!(out, "from {} import (", i.module);
                for n in &i.names {
                    let _ = writeln!(out, "    {n},");
                }
                out.push_str(")\n");
            }
        }
    }
    out
}

/// A type expression.
fn py_type(ty: &Type) -> String {
    match ty {
        Type::Ref { name } => name.clone(),
        Type::Int64 => "Int64".into(),
        Type::Integer { .. } => "int".into(),
        Type::Number => "float".into(),
        Type::String => "str".into(),
        Type::Bool => "bool".into(),
        Type::Array { item } => format!("list[{}]", py_type(item)),
        Type::Map { value } => format!("dict[str, {}]", py_type(value)),
        Type::Enum { values } => {
            let quoted: Vec<String> = values.iter().map(|v| py_string(v)).collect();
            format!("Literal[{}]", quoted.join(", "))
        }
        _ => "Any".into(),
    }
}

/// A Python string literal.
fn py_string(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// A docstring's text: one line, no closing quotes inside.
fn doc_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace("\"\"\"", "\\\"\\\"\\\"")
        .replace('\n', " ")
        .trim()
        .to_owned()
}

/// The typing names, runtime names and other models a type mentions.
#[derive(Default)]
struct Uses {
    typing: BTreeSet<&'static str>,
    abc: BTreeSet<&'static str>,
    runtime: BTreeSet<String>,
    models: BTreeSet<String>,
}

impl Uses {
    fn ty(&mut self, ty: &Type) {
        match ty {
            Type::Ref { name } => {
                if ir::RUNTIME_TYPES.contains(&name.as_str()) {
                    self.runtime.insert(name.clone());
                } else {
                    self.models.insert(name.clone());
                }
            }
            Type::Int64 => {
                self.runtime.insert("Int64".into());
            }
            Type::Array { item } => self.ty(item),
            Type::Map { value } => self.ty(value),
            Type::Enum { .. } => {
                self.typing.insert("Literal");
            }
            Type::String | Type::Integer { .. } | Type::Number | Type::Bool => {}
            _ => {
                self.typing.insert("Any");
            }
        }
    }
}

/// A model field's Python name, and the wire name when they differ.
fn field_name(wire: &str) -> (String, Option<String>) {
    let snake = wire.to_snake_case();
    let clash = KEYWORDS.contains(&snake.as_str())
        || MODEL_ATTRIBUTES.contains(&snake.as_str())
        || snake.starts_with("model_");
    let name = if clash { format!("{snake}_") } else { snake };
    if name == wire {
        (name, None)
    } else {
        (name, Some(wire.to_owned()))
    }
}

fn render_field(f: &Field, uses: &mut Uses) -> String {
    uses.ty(&f.ty);
    let (name, alias) = field_name(&f.name);
    let mut ty = py_type(&f.ty);
    if f.nullable || !f.required {
        ty.push_str(" | None");
    }
    let mut out = String::new();
    match (alias, f.required) {
        (Some(wire), true) => {
            let _ = writeln!(out, "    {name}: {ty} = _Field(alias={})", py_string(&wire));
        }
        (Some(wire), false) => {
            let _ = writeln!(
                out,
                "    {name}: {ty} = _Field(default=None, alias={})",
                py_string(&wire)
            );
        }
        (None, true) => {
            let _ = writeln!(out, "    {name}: {ty}");
        }
        (None, false) => {
            let _ = writeln!(out, "    {name}: {ty} = None");
        }
    }
    if !f.doc.is_empty() {
        let _ = writeln!(out, "    \"\"\"{}\"\"\"", doc_text(&f.doc));
    }
    out
}

/// The models: classes first, then the type aliases (which may name a class), so the
/// module imports without a forward reference; the classes resolve theirs at the end.
fn render_models(models: &[Model], runtime: &str) -> String {
    let mut uses = Uses::default();
    let mut classes = String::new();
    let mut class_names = Vec::new();
    let mut aliases: Vec<(&Model, String, BTreeSet<String>)> = Vec::new();
    for m in models.iter().filter(|m| !m.runtime) {
        match &m.shape {
            Shape::Object { fields } => {
                class_names.push(m.name.clone());
                let _ = writeln!(classes, "\n\nclass {}(_Model):", m.name);
                if !m.doc.is_empty() {
                    let _ = writeln!(classes, "    \"\"\"{}\"\"\"", doc_text(&m.doc));
                }
                if fields.is_empty() {
                    if m.doc.is_empty() {
                        classes.push_str("    pass\n");
                    }
                } else {
                    if !m.doc.is_empty() {
                        classes.push('\n');
                    }
                    for f in fields {
                        classes.push_str(&render_field(f, &mut uses));
                    }
                }
            }
            Shape::Enum { values } => {
                uses.typing.insert("Literal");
                let ty = py_type(&Type::Enum {
                    values: values.clone(),
                });
                aliases.push((m, ty, BTreeSet::new()));
            }
            Shape::Union { variants, .. } => {
                let mut inner = Uses::default();
                for v in variants {
                    uses.ty(v);
                    inner.ty(v);
                }
                let joined: Vec<String> = variants.iter().map(py_type).collect();
                aliases.push((m, joined.join(" | "), inner.models));
            }
            Shape::Alias { ty } => {
                let mut inner = Uses::default();
                uses.ty(ty);
                inner.ty(ty);
                aliases.push((m, py_type(ty), inner.models));
            }
        }
    }
    let ordered = order_aliases(&aliases);
    let mut alias_text = String::new();
    if !ordered.is_empty() {
        uses.typing.insert("TypeAlias");
    }
    for (m, ty) in &ordered {
        let _ = writeln!(alias_text, "\n{}: TypeAlias = {ty}", m.name);
        if !m.doc.is_empty() {
            let _ = writeln!(alias_text, "\"\"\"{}\"\"\"", doc_text(&m.doc));
        }
    }

    let mut out = String::from(
        "\"\"\"The models of the InOrbit API this surface was generated from.\"\"\"\n\n",
    );
    let pydantic = vec!["BaseModel".to_owned(), "ConfigDict".to_owned()];
    // pydantic's Field is imported under a private name: the API may have a model called
    // Field (it does since 2026-10-03), and a model must never shadow the helper.
    let field_helper = classes.contains("_Field(");
    out.push_str(&imports(
        vec![Import::new(
            "typing",
            uses.typing.iter().map(|t| (*t).to_owned()),
        )],
        vec![
            Import::new("pydantic", pydantic),
            // An aliased name is its own statement, as isort (ruff) orders them.
            Import::new(
                "pydantic",
                field_helper.then(|| "Field as _Field".to_owned()),
            ),
            Import::new(runtime, uses.runtime.iter().cloned()),
        ],
    ));
    out.push_str(
        "\n\nclass _Model(BaseModel):\n    \"\"\"Fields this version does not know are kept; wire names are accepted too.\"\"\"\n\n    model_config = ConfigDict(extra=\"allow\", populate_by_name=True)\n",
    );
    out.push_str(&classes);
    out.push_str(&alias_text);
    if !class_names.is_empty() {
        out.push_str("\n\nfor _model in (\n");
        for name in &class_names {
            let _ = writeln!(out, "    {name},");
        }
        out.push_str("):\n    _model.model_rebuild()\n");
    }
    out
}

/// The aliases in an order where each comes after the aliases it names.
fn order_aliases<'a>(
    aliases: &[(&'a Model, String, BTreeSet<String>)],
) -> Vec<(&'a Model, String)> {
    let alias_names: BTreeSet<String> = aliases.iter().map(|(m, _, _)| m.name.clone()).collect();
    let mut placed: BTreeSet<String> = BTreeSet::new();
    let mut ordered = Vec::new();
    while ordered.len() < aliases.len() {
        let before = ordered.len();
        for (m, ty, deps) in aliases {
            if placed.contains(&m.name) {
                continue;
            }
            if deps
                .iter()
                .all(|d| !alias_names.contains(d) || placed.contains(d))
            {
                placed.insert(m.name.clone());
                ordered.push((*m, ty.clone()));
            }
        }
        if ordered.len() == before {
            // A cycle between aliases: keep name order; pydantic reports it on use.
            for (m, ty, _) in aliases {
                if placed.insert(m.name.clone()) {
                    ordered.push((*m, ty.clone()));
                }
            }
        }
    }
    ordered
}

/// The function that builds an operation, named after its marker, which is unique.
fn builder_name(op: &Op) -> String {
    op.marker.to_snake_case()
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

fn op_doc(op: &Op) -> String {
    let mut text = format!("`{}`{}.", op.line, scopes_note(op));
    if !op.doc.is_empty() {
        let _ = write!(text, " {}", op.doc);
    }
    doc_text(&text)
}

/// The parameters an operation takes, without `self`: path parameters and the body
/// positional, the query parameters keyword-only.
fn parameters(op: &Op, uses: &mut Uses) -> Vec<String> {
    let naming = PyNaming;
    let mut out = Vec::new();
    for p in &op.path_params {
        out.push(format!("{}: str", naming.field_name(&p.name)));
    }
    if let Some(body) = &op.body {
        uses.models.insert(body.clone());
        out.push(format!("body: {body}"));
    }
    let mut keyword = Vec::new();
    for q in &op.query {
        uses.ty(&q.ty);
        let name = naming.field_name(&q.name);
        let ty = py_type(&q.ty);
        if q.required {
            keyword.push(format!("{name}: {ty}"));
        } else {
            keyword.push(format!("{name}: {ty} | None = None"));
        }
    }
    if !keyword.is_empty() {
        out.push("*".into());
        out.extend(keyword);
    }
    out
}

/// The names the builder is called with, in its own order.
fn call_arguments(op: &Op) -> Vec<String> {
    let naming = PyNaming;
    let mut out: Vec<String> = op
        .path_params
        .iter()
        .map(|p| naming.field_name(&p.name))
        .collect();
    if op.body.is_some() {
        out.push("body".into());
    }
    for q in &op.query {
        let name = naming.field_name(&q.name);
        out.push(format!("{name}={name}"));
    }
    out
}

fn render_operations(surface: &context::Surface, runtime: &str) -> String {
    let naming = PyNaming;
    let mut sorted: Vec<&Op> = surface
        .handles
        .iter()
        .flat_map(|h| h.ops.iter())
        .chain(surface.flat.iter())
        .collect();
    sorted.sort_by(|a, b| a.line.cmp(&b.line));
    let mut uses = Uses::default();
    let mut body = String::new();
    for op in &sorted {
        let params = parameters(op, &mut uses);
        let path: String = op
            .segments
            .iter()
            .map(|s| match s {
                Segment::Literal { text } => text.replace('{', "{{").replace('}', "}}"),
                Segment::Param { name } => {
                    format!("{{codegen.path_segment({})}}", naming.field_name(name))
                }
            })
            .collect();
        let path_expr = if op.path_params.is_empty() {
            py_string(&op.path)
        } else {
            format!("f{}", py_string(&path))
        };
        let _ = write!(
            body,
            "\n\ndef {}({}) -> Operation:\n    \"\"\"{}\"\"\"\n    return Operation(\n        name={},\n        method={},\n        path={path_expr},\n",
            builder_name(op),
            params.join(", "),
            op_doc(op),
            py_string(&op.hook_name),
            py_string(op.method.as_str()),
        );
        if !op.query.is_empty() {
            let pairs: Vec<String> = op
                .query
                .iter()
                .map(|q| format!("({}, {})", py_string(&q.name), naming.field_name(&q.name)))
                .collect();
            let _ = writeln!(body, "        query=({},),", pairs.join(", "));
        }
        if op.body.is_some() {
            body.push_str("        body=body,\n");
        }
        let scopes: Vec<String> = op.scopes.iter().map(|s| py_string(s)).collect();
        if !scopes.is_empty() {
            let _ = writeln!(body, "        scopes=({},),", scopes.join(", "));
        }
        if op.idempotent_override {
            body.push_str("        idempotent=True,\n");
        }
        if op.stream {
            // A stream's call on the socket names its RPC and takes its path parameters
            // as fields, unencoded (design.md section 7).
            if !op.rpc.is_empty() {
                let _ = writeln!(body, "        rpc={},", py_string(&op.rpc));
            }
            if !op.path_params.is_empty() {
                let pairs: Vec<String> = op
                    .path_params
                    .iter()
                    .map(|p| format!("({}, {})", py_string(&p.name), naming.field_name(&p.name)))
                    .collect();
                let _ = writeln!(body, "        params=({},),", pairs.join(", "));
            }
        }
        body.push_str("    )\n");
    }
    let mut out = String::from(
        "\"\"\"Each operation of this surface as the runtime's `Operation`, built from its arguments.\"\"\"\n\n",
    );
    let mut runtime_names: BTreeSet<String> = uses.runtime.clone();
    runtime_names.insert("Operation".into());
    runtime_names.insert("codegen".into());
    out.push_str(&imports(
        vec![Import::new(
            "typing",
            uses.typing.iter().map(|t| (*t).to_owned()),
        )],
        vec![
            Import::new(runtime, runtime_names),
            Import::new(".models", uses.models.iter().cloned()),
        ],
    ));
    out.push_str(&body);
    out
}

/// One profile in one form.
#[derive(Clone, Copy)]
enum Form {
    Sync,
    Async,
}

impl Form {
    fn prefix(self) -> &'static str {
        match self {
            Self::Sync => "",
            Self::Async => "Async",
        }
    }

    fn client(self) -> &'static str {
        match self {
            Self::Sync => "Client",
            Self::Async => "AsyncClient",
        }
    }
}

fn render_profiles(surface: &context::Surface, models: &[Model], runtime: &str) -> String {
    let mut uses = Uses::default();
    let mut body = String::new();
    for p in &surface.profiles {
        for form in [Form::Sync, Form::Async] {
            body.push_str(&profile_class(surface, p, models, form, &mut uses));
        }
    }
    let mut out = String::from(
        "\"\"\"One class per profile, blocking and `asyncio`: each holds only the operations its cut holds.\"\"\"\n\n",
    );
    let mut typing: Vec<String> = uses.typing.iter().map(|t| (*t).to_owned()).collect();
    typing.push("ClassVar".into());
    let mut runtime_names: BTreeSet<String> = uses.runtime.clone();
    for n in ["AsyncClient", "Client", "Response"] {
        runtime_names.insert(n.into());
    }
    out.push_str(&imports(
        vec![
            Import::new("collections.abc", uses.abc.iter().map(|t| (*t).to_owned())),
            Import::new("typing", typing),
        ],
        vec![
            Import::new(runtime, runtime_names),
            Import::new(".", ["operations as _ops".to_owned()]),
            Import::new(".models", uses.models.iter().cloned()),
        ],
    ));
    out.push_str(&body);
    out
}

/// One profile's class and its handle classes, in one form.
fn profile_class(
    surface: &context::Surface,
    p: &context::Profile,
    models: &[Model],
    form: Form,
    uses: &mut Uses,
) -> String {
    let naming = PyNaming;
    let class = format!("{}{}", form.prefix(), naming.type_name(&p.name));
    let client = form.client();
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
        "`from_env()` reads `INORBIT_TOKEN`, or `INORBIT_KEY_ID`, `INORBIT_KEY_SECRET` and `INORBIT_SCOPES`".to_owned()
    } else {
        format!(
            "`from_env()` reads `INORBIT_{e}_TOKEN`, or `INORBIT_{e}_KEY_ID`, `INORBIT_{e}_KEY_SECRET` and `INORBIT_{e}_SCOPES`, and nothing else",
            e = p.env
        )
    };
    let mut out = format!(
        "\n\nclass {class}:\n    \"\"\"Profile `{}`: the operations its cut holds. {env_doc}.\"\"\"\n\n    profile: ClassVar[str] = {}\n    \"\"\"The profile's name.\"\"\"\n",
        p.name,
        py_string(&p.name)
    );
    for (h, _) in &handles {
        let _ = write!(
            out,
            "    {}: {class}{}\n    \"\"\"The `{}` operations.\"\"\"\n",
            naming.method_name(&h.tag),
            naming.type_name(&h.tag),
            h.tag
        );
    }
    let _ = write!(
        out,
        "\n    def __init__(self, client: {client}) -> None:\n        \"\"\"The profile on a client built for its credential.\"\"\"\n        self.client = client\n"
    );
    for (h, _) in &handles {
        let _ = writeln!(
            out,
            "        self.{} = {class}{}(client)",
            naming.method_name(&h.tag),
            naming.type_name(&h.tag)
        );
    }
    let env_arg = if p.is_public {
        String::new()
    } else {
        py_string(&p.env)
    };
    let _ = write!(
        out,
        "\n    @classmethod\n    def from_env(cls) -> {class}:\n        \"\"\"The profile with its credential from the environment.\"\"\"\n        return cls({client}.from_env({env_arg}))\n"
    );
    for op in surface.flat.iter().filter(|o| o.profiles.contains(&p.name)) {
        out.push_str(&profile_method(op, models, "self.client", form, uses));
    }
    for (h, ops) in &handles {
        let _ = write!(
            out,
            "\n\nclass {class}{}:\n    \"\"\"The `{}` operations profile `{}` may call.\"\"\"\n\n    def __init__(self, client: {client}) -> None:\n        \"\"\"The handle on a profile's client.\"\"\"\n        self._client = client\n",
            naming.type_name(&h.tag),
            h.tag,
            p.name
        );
        for op in ops {
            out.push_str(&profile_method(op, models, "self._client", form, uses));
        }
    }
    out
}

fn profile_method(op: &Op, models: &[Model], client: &str, form: Form, uses: &mut Uses) -> String {
    if op.stream {
        return stream_method(op, client, form, uses);
    }
    let naming = PyNaming;
    let mut params = parameters(op, uses);
    if !params.contains(&"*".to_owned()) {
        params.push("*".into());
    }
    params.push("timeout: float | None = None".into());
    let response = op.response.clone().unwrap_or_else(|| "object".into());
    if let Some(r) = &op.response {
        uses.models.insert(r.clone());
    }
    let (def, call) = match form {
        Form::Sync => ("def", ""),
        Form::Async => ("async def", "await "),
    };
    let mut out = format!(
        "\n    {def} {}(self, {}) -> Response[{response}]:\n        \"\"\"{}\"\"\"\n        return {call}{client}.request(\n            _ops.{}({}),\n            {response},\n            timeout=timeout,\n        )\n",
        naming.method_name(&op.name),
        params.join(", "),
        op_doc(op),
        builder_name(op),
        call_arguments(op).join(", ")
    );
    if let Some(paging) = context::paging(op, models) {
        out.push_str(&iterator_method(op, &paging, &params, form, uses));
    }
    out
}

/// A streaming operation: a method that hands back the runtime's `Stream` (blocking) or
/// `AsyncStream` (`asyncio`), which open on the first step of the iteration and yield the
/// answer's model once per event (design.md section 7).
fn stream_method(op: &Op, client: &str, form: Form, uses: &mut Uses) -> String {
    let naming = PyNaming;
    let params = parameters(op, uses);
    let item = op.response.clone().unwrap_or_else(|| "object".into());
    if let Some(r) = &op.response {
        uses.models.insert(r.clone());
    }
    let stream = match form {
        Form::Sync => "Stream",
        Form::Async => "AsyncStream",
    };
    uses.runtime.insert(stream.into());
    let mut signature = vec!["self".to_owned()];
    signature.extend(params);
    format!(
        "\n    def {}({}) -> {stream}[{item}]:\n        \"\"\"{}\"\"\"\n        return {client}.stream(_ops.{}({}), {item})\n",
        naming.method_name(&op.name),
        signature.join(", "),
        op_doc(op),
        builder_name(op),
        call_arguments(op).join(", ")
    )
}

/// `all_<op>`: every item of a paged list, page after page: a generator in the blocking
/// class, an async generator (`async for`) in the asyncio one (design.md §9).
fn iterator_method(
    op: &Op,
    paging: &context::Paging,
    params: &[String],
    form: Form,
    uses: &mut Uses,
) -> String {
    let naming = PyNaming;
    uses.ty(&paging.item);
    uses.runtime.insert("codegen".into());
    let item = py_type(&paging.item);
    let method = naming.method_name(&op.name);
    let token = naming.field_name(&paging.token_param);
    // The page call: the operation's own method, every argument passed through, the
    // token replaced once there is one (the caller's own token starts the walk).
    let mut args: Vec<String> = op
        .path_params
        .iter()
        .map(|p| naming.field_name(&p.name))
        .collect();
    if op.body.is_some() {
        args.push("body".into());
    }
    for q in &op.query {
        let name = naming.field_name(&q.name);
        if q.name == paging.token_param {
            args.push(format!("{name}=page if page is not None else {token}"));
        } else {
            args.push(format!("{name}={name}"));
        }
    }
    args.push("timeout=timeout".into());
    let (list, _) = field_name(&paging.list_field);
    let (next, _) = field_name(&paging.next_field);
    let doc = format!(
        "Every item `{}` answers, page after page, following `{}` until the last page.",
        op.line, paging.next_field
    );
    let (def, iter, fetch_def, wait, helper) = match form {
        Form::Sync => ("def", "Iterator", "def", "", "pages"),
        Form::Async => ("def", "AsyncIterator", "async def", "await ", "apages"),
    };
    uses.abc.insert(iter);
    format!(
        "\n    {def} all_{method}(self, {}) -> {iter}[{item}]:\n        \"\"\"{doc}\"\"\"\n\n        {fetch_def} fetch(page: str | None) -> tuple[list[{item}], str]:\n            value = ({wait}self.{method}({})).value\n            return value.{list} or [], value.{next} or \"\"\n\n        return codegen.{helper}(fetch)\n",
        params.join(", "),
        args.join(", "),
    )
}

fn render_init(surface: &context::Surface, models: &[Model], runtime: &str, root: &str) -> String {
    let naming = PyNaming;
    let names: Vec<String> = surface
        .profiles
        .iter()
        .map(|p| p.name.as_str())
        .map(|n| format!("`{n}`"))
        .collect();
    let mut out = format!(
        "\"\"\"The InOrbit API surface for {} (API {}).\n\nGenerated by `iohr sdk generate`: the models and the operations these profiles may call,\non the `{root}` runtime. Regenerate it, never edit it; `iohr sdk check` says when the\nAPI's cut has moved.\n\"\"\"\n\n",
        names.join(", "),
        surface.api_version
    );
    let model_names: Vec<String> = models
        .iter()
        .filter(|m| !m.runtime)
        .map(|m| m.name.clone())
        .collect();
    let mut profile_names = Vec::new();
    for p in &surface.profiles {
        let base = naming.type_name(&p.name);
        profile_names.push(format!("Async{base}"));
        profile_names.push(base);
    }
    out.push_str(&imports(
        Vec::new(),
        vec![
            Import::new(runtime, ["codegen".to_owned()]),
            Import::new(".models", model_names.iter().cloned()),
            Import::new(".profiles", profile_names.iter().cloned()),
        ],
    ));
    out.push_str(
        "\n# A surface generated for another runtime contract refuses to import.\ncodegen.check(1)\n\n__all__ = [\n",
    );
    let mut all: Vec<&String> = model_names.iter().chain(profile_names.iter()).collect();
    all.sort();
    for n in all {
        let _ = writeln!(out, "    {},", py_string(n));
    }
    out.push_str("]\n");
    out
}
