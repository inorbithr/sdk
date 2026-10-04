//! The Java target: a record per model, a record (with a builder) per operation's query
//! parameters, `Operations` with one static method per operation over the runtime's one
//! request path, and one class per profile whose handles hold only the operations its cut
//! holds, so a call the profile may not make does not compile.
//!
//! One public type per file, as Java wants, all in one package (`--package`; the runtime's
//! own surface is `hr.inorbit.sdk.generated`). Models carry their wire names in
//! `@JsonProperty`; a 64-bit integer is a `long` written as a decimal string; a field the
//! answer may leave out is a boxed, nullable type. The generated code compiles with
//! `-Xlint:all -Xdoclint:all/protected -Werror`, so every public element has Javadoc.

mod example;
pub(crate) use example::example;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use heck::{ToLowerCamelCase as _, ToShoutySnakeCase as _};
use iohr_openapi::Api;

use crate::context::{self, Naming, Op, Segment};
use crate::files::Files;
use crate::ir::{Field, Model, Shape, Type};
use crate::language::{Language, Options};
use crate::target::{RenderError, Target};

/// The Java target.
#[derive(Debug, Clone, Copy, Default)]
pub struct JavaTarget;

/// How Java spells names: `lowerCamel` methods and fields, `UpperCamel` types, and a
/// trailing `_` on a keyword or on a name a record component cannot take (an `Object`
/// method such as `hashCode`).
struct JavaNaming;

/// Java's keywords and restricted identifiers.
const KEYWORDS: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "float",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "record",
    "return",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "true",
    "try",
    "var",
    "void",
    "volatile",
    "while",
    "yield",
];

/// Names a record component or a profile's method cannot take: an `Object` method, and
/// what the profile classes use themselves.
const TAKEN: &[&str] = &[
    "clone",
    "finalize",
    "getClass",
    "hashCode",
    "notify",
    "notifyAll",
    "toString",
    "wait",
    "client",
    "builder",
];

impl Naming for JavaNaming {
    const RESERVED: &'static [&'static str] = KEYWORDS;

    fn escape(&self, ident: String) -> String {
        if KEYWORDS.contains(&ident.as_str()) || TAKEN.contains(&ident.as_str()) {
            format!("{ident}_")
        } else {
            ident
        }
    }

    fn method_name(&self, snake: &str) -> String {
        self.escape(snake.to_lower_camel_case())
    }

    fn field_name(&self, wire: &str) -> String {
        self.escape(wire.to_lower_camel_case())
    }
}

/// The runtime's types a surface refers to, by simple name.
const RUNTIME: &[&str] = &[
    "Client",
    "Response",
    "Operation",
    "Method",
    "Code",
    "Detail",
    "Pages",
    "EventStream",
];

impl Target for JavaTarget {
    const LANG: Language = Language::Java;

    fn render(&self, api: &Api, options: &Options) -> Result<Files, RenderError> {
        let runtime = options.runtime_for(Self::LANG);
        let package = match (&options.package, options.in_package) {
            (Some(p), _) => p.clone(),
            (None, true) => format!("{runtime}.generated"),
            (None, false) => "iohr".to_owned(),
        };
        check_package(&package)?;
        let surface = context::surface(api);
        let models = context::models(api);
        let out = Gen {
            header: format!("// {}\n", surface.header),
            package,
            runtime,
            types: models.iter().map(|m| (m.name.clone(), m.clone())).collect(),
            models: models.clone(),
        };
        let mut files = Files::new();
        for note in &surface.notes {
            files.note(note.clone());
        }
        let bodies: BTreeSet<String> = surface
            .handles
            .iter()
            .flat_map(|h| h.ops.iter())
            .chain(surface.flat.iter())
            .filter_map(|o| o.body.clone())
            .collect();
        for m in models.iter().filter(|m| !m.runtime) {
            if let Some(text) = out.model(m, bodies.contains(&m.name)) {
                files.insert(format!("{}.java", m.name), text);
            }
        }
        let mut all: Vec<&Op> = surface
            .handles
            .iter()
            .flat_map(|h| h.ops.iter())
            .chain(surface.flat.iter())
            .collect();
        all.sort_by(|a, b| a.line.cmp(&b.line));
        for op in &all {
            if let Some(t) = &op.params_type {
                files.insert(format!("{t}.java"), out.params(t, op));
            }
        }
        files.insert("Operations.java", out.operations(&all, &surface));
        for p in &surface.profiles {
            for (name, text) in out.profile(&surface, p) {
                files.insert(format!("{name}.java"), text);
            }
        }
        files.insert("package-info.java", out.package_info(&surface));
        Ok(files)
    }
}

/// A package name Java accepts: dot-separated identifiers, none a keyword.
fn check_package(package: &str) -> Result<(), RenderError> {
    let ok = !package.is_empty()
        && package.split('.').all(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !KEYWORDS.contains(&part)
        });
    if ok {
        Ok(())
    } else {
        Err(RenderError::Models(format!(
            "{package:?} is not a Java package name; pass --package such as com.example.iohr"
        )))
    }
}

/// Javadoc text: HTML escaped, backticks as `<code>`, never closing the comment.
fn jdoc(text: &str) -> String {
    let mut out = String::new();
    let mut code = false;
    for c in text.chars() {
        match c {
            '`' => {
                out.push_str(if code { "</code>" } else { "<code>" });
                code = !code;
            }
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '@' => out.push_str("&#64;"),
            '*' => out.push_str("&#42;"),
            '\n' | '\r' => out.push(' '),
            c => out.push(c),
        }
    }
    if code {
        out.push_str("</code>");
    }
    out.trim().to_owned()
}

/// A Java string literal.
fn lit(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A Javadoc block at `indent`: the main text, then the tags.
fn javadoc(indent: &str, main: &str, tags: &[String]) -> String {
    let mut out = format!("{indent}/**\n");
    let _ = writeln!(out, "{indent} * {main}");
    if !tags.is_empty() {
        let _ = writeln!(out, "{indent} *");
        for t in tags {
            let _ = writeln!(out, "{indent} * {t}");
        }
    }
    let _ = writeln!(out, "{indent} */");
    out
}

/// The imports a file needs, collected while it is written.
#[derive(Default)]
struct Imports(BTreeSet<String>);

impl Imports {
    fn add(&mut self, fqn: impl Into<String>) {
        self.0.insert(fqn.into());
    }

    fn render(&self) -> String {
        let mut out = String::new();
        for i in &self.0 {
            let _ = writeln!(out, "import {i};");
        }
        out
    }
}

struct Gen {
    header: String,
    package: String,
    runtime: String,
    types: BTreeMap<String, Model>,
    models: Vec<Model>,
}

impl Gen {
    fn file(&self, imports: &Imports, body: &str) -> String {
        let mut out = format!("{}package {};\n\n", self.header, self.package);
        let list = imports.render();
        if !list.is_empty() {
            out.push_str(&list);
            out.push('\n');
        }
        out.push_str(body);
        out
    }

    /// A runtime type by simple name, imported.
    fn rt(&self, name: &str, imports: &mut Imports) -> String {
        debug_assert!(RUNTIME.contains(&name));
        let fqn = match name {
            "Code" | "Detail" => format!("{}.errors.{name}", self.runtime),
            _ => format!("{}.{name}", self.runtime),
        };
        imports.add(fqn);
        name.to_owned()
    }

    /// A type expression: primitive when `boxed` is false and the type has one.
    fn ty(&self, ty: &Type, boxed: bool, imports: &mut Imports) -> String {
        match ty {
            Type::Ref { name } => match self.types.get(name) {
                Some(m) if m.runtime => self.rt(name, imports),
                Some(Model {
                    shape: Shape::Alias { ty },
                    ..
                }) => self.ty(ty, boxed, imports),
                Some(Model {
                    shape: Shape::Union { .. },
                    ..
                })
                | None => {
                    imports.add("com.fasterxml.jackson.databind.JsonNode");
                    "JsonNode".into()
                }
                Some(_) => name.clone(),
            },
            Type::String | Type::Enum { .. } => "String".into(),
            Type::Int64 | Type::Integer { bits: 64 } => if boxed { "Long" } else { "long" }.into(),
            Type::Integer { .. } => if boxed { "Integer" } else { "int" }.into(),
            Type::Number => if boxed { "Double" } else { "double" }.into(),
            Type::Bool => if boxed { "Boolean" } else { "boolean" }.into(),
            Type::Array { item } => {
                imports.add("java.util.List");
                format!("List<{}>", self.ty(item, true, imports))
            }
            Type::Map { value } => {
                imports.add("java.util.Map");
                format!("Map<String, {}>", self.ty(value, true, imports))
            }
            _ => {
                imports.add("com.fasterxml.jackson.databind.JsonNode");
                "JsonNode".into()
            }
        }
    }

    /// Whether `ty` holds a 64-bit integer the wire carries as a decimal string, and where.
    fn int64(ty: &Type) -> Int64At {
        match ty {
            Type::Int64 => Int64At::Value,
            Type::Array { item } | Type::Map { value: item } if matches!(**item, Type::Int64) => {
                Int64At::Contents
            }
            _ => Int64At::None,
        }
    }

    fn model(&self, m: &Model, builder: bool) -> Option<String> {
        match &m.shape {
            Shape::Object { fields } => Some(self.record(m, fields, builder)),
            Shape::Enum { values } => Some(self.enumeration(m, values)),
            // A union or an alias has no class of its own: a use of it is the type it names,
            // or `JsonNode` for a union.
            Shape::Union { .. } | Shape::Alias { .. } => None,
        }
    }

    fn record(&self, m: &Model, fields: &[Field], builder: bool) -> String {
        let naming = JavaNaming;
        let mut imports = Imports::default();
        imports.add("com.fasterxml.jackson.annotation.JsonIgnoreProperties");
        let mut components = Vec::new();
        let mut tags = Vec::new();
        let mut typed = Vec::new();
        for f in fields {
            let name = naming.field_name(&f.name);
            let boxed = !f.required || f.nullable;
            let ty = self.ty(&f.ty, boxed, &mut imports);
            imports.add("com.fasterxml.jackson.annotation.JsonProperty");
            let mut annotations = format!("@JsonProperty({})", lit(&f.name));
            match Self::int64(&f.ty) {
                Int64At::Value => {
                    imports.add("com.fasterxml.jackson.annotation.JsonFormat");
                    annotations.push_str(" @JsonFormat(shape = JsonFormat.Shape.STRING)");
                }
                Int64At::Contents => {
                    imports.add("com.fasterxml.jackson.databind.annotation.JsonSerialize");
                    imports.add("com.fasterxml.jackson.databind.ser.std.ToStringSerializer");
                    annotations
                        .push_str(" @JsonSerialize(contentUsing = ToStringSerializer.class)");
                }
                Int64At::None => {}
            }
            components.push(format!("        {annotations} {ty} {name}"));
            let mut doc = if f.doc.is_empty() {
                format!("the <code>{}</code> field", jdoc(&f.name))
            } else {
                jdoc(&f.doc)
            };
            if boxed {
                doc.push_str("; <code>null</code> when unset, and then left out on the wire");
            }
            tags.push(format!("@param {name} {doc}"));
            typed.push((name, ty));
        }
        let main = if m.doc.is_empty() {
            format!("The <code>{}</code> model.", m.name)
        } else {
            jdoc(&m.doc)
        };
        let mut body = javadoc("", &main, &tags);
        body.push_str("@JsonIgnoreProperties(ignoreUnknown = true)\n");
        if components.is_empty() {
            let _ = writeln!(body, "public record {}() {{", m.name);
        } else {
            let _ = writeln!(
                body,
                "public record {}(\n{}) {{",
                m.name,
                components.join(",\n")
            );
        }
        if builder && !typed.is_empty() {
            body.push_str(&builder_class(&m.name, &typed));
            body.push_str("}\n");
        } else {
            // An empty body on the declaration's own line, as Java style has it.
            body.truncate(body.len() - 3);
            body.push_str(" {}\n");
        }
        self.file(&imports, &body)
    }

    fn enumeration(&self, m: &Model, values: &[String]) -> String {
        let mut imports = Imports::default();
        imports.add("com.fasterxml.jackson.annotation.JsonEnumDefaultValue");
        imports.add("com.fasterxml.jackson.annotation.JsonProperty");
        let main = if m.doc.is_empty() {
            format!("The <code>{}</code> values.", m.name)
        } else {
            jdoc(&m.doc)
        };
        let mut body = javadoc("", &main, &[]);
        let _ = writeln!(body, "public enum {} {{", m.name);
        let mut used = BTreeSet::new();
        for v in values {
            let mut constant = v.to_shouty_snake_case();
            if constant.is_empty() || constant.starts_with(|c: char| c.is_ascii_digit()) {
                constant = format!("V_{constant}");
            }
            while !used.insert(constant.clone()) {
                constant.push('_');
            }
            let _ = write!(
                body,
                "    /** <code>{}</code>. */\n    @JsonProperty({})\n    {constant},\n",
                jdoc(v),
                lit(v)
            );
        }
        body.push_str(
            "    /** A value this version of the surface does not know. */\n    @JsonEnumDefaultValue\n    UNRECOGNIZED\n}\n",
        );
        self.file(&imports, &body)
    }

    fn params(&self, type_name: &str, op: &Op) -> String {
        let naming = JavaNaming;
        let mut imports = Imports::default();
        let mut components = Vec::new();
        let mut tags = Vec::new();
        let mut typed = Vec::new();
        for q in &op.query {
            let name = naming.field_name(&q.name);
            let ty = self.ty(&q.ty, true, &mut imports);
            components.push(format!("        {ty} {name}"));
            let mut doc = if q.doc.is_empty() {
                format!("<code>{}</code>", jdoc(&q.name))
            } else {
                jdoc(&q.doc)
            };
            if !q.required {
                doc.push_str("; <code>null</code> to leave it out");
            }
            tags.push(format!("@param {name} {doc}"));
            typed.push((name, ty));
        }
        let mut body = javadoc(
            "",
            &format!("The query parameters of <code>{}</code>.", jdoc(&op.line)),
            &tags,
        );
        let _ = writeln!(
            body,
            "public record {type_name}(\n{}) {{",
            components.join(",\n")
        );
        body.push_str(&builder_class(type_name, &typed));
        body.push_str("}\n");
        self.file(&imports, &body)
    }

    /// One operation's static method and its async twin.
    #[allow(clippy::too_many_lines)] // one operation's rendering, unary and stream, read top to bottom
    fn operation(&self, op: &Op, imports: &mut Imports) -> String {
        let naming = JavaNaming;
        let client = self.rt("Client", imports);
        let response = self.rt("Response", imports);
        let answer = self.answer(op, imports);
        let (decl, tags, _) = self.arguments(op, imports);
        let fname = function_name(op);
        let mut path = String::new();
        let mut parts = Vec::new();
        for s in &op.segments {
            match s {
                Segment::Literal { text } => parts.push(lit(text)),
                Segment::Param { name } => {
                    imports.add(format!("{}.codegen.Codegen", self.runtime));
                    parts.push(format!("Codegen.pathSegment({})", naming.field_name(name)));
                }
            }
        }
        path.push_str(&parts.join(" + "));
        let method = self.rt("Method", imports);
        let operation = self.rt("Operation", imports);
        let mut build = format!(
            "{operation}.builder({method}.{}, {path})\n                .name({})",
            op.method.as_str(),
            lit(&op.hook_name)
        );
        for q in &op.query {
            let _ = write!(
                build,
                "\n                .query({}, params.{}())",
                lit(&q.name),
                naming.field_name(&q.name)
            );
        }
        if op.body.is_some() {
            build.push_str("\n                .body(body)");
        }
        if !op.scopes.is_empty() {
            let scopes: Vec<String> = op.scopes.iter().map(|s| lit(s)).collect();
            let _ = write!(build, "\n                .scopes({})", scopes.join(", "));
        }
        if op.idempotent_override {
            build.push_str("\n                .idempotent(true)");
        }
        if op.stream {
            // The socket's call frame carries the parameters typed, by wire name.
            for p in &op.path_params {
                let _ = write!(
                    build,
                    "\n                .field({}, {})",
                    lit(&p.name),
                    naming.field_name(&p.name)
                );
            }
            for q in &op.query {
                let _ = write!(
                    build,
                    "\n                .field({}, params.{}())",
                    lit(&q.name),
                    naming.field_name(&q.name)
                );
            }
            if !op.rpc.is_empty() {
                let _ = write!(build, "\n                .rpc({})", lit(&op.rpc));
            }
        }
        build.push_str("\n                .build()");
        let mut all_tags =
            vec!["@param client the client of a profile that may call it".to_owned()];
        all_tags.extend(tags);
        if op.stream {
            let stream = self.rt("EventStream", imports);
            all_tags.push("@return the events, one model each; close it to stop early".into());
            let args = std::iter::once(format!("{client} client"))
                .chain(decl)
                .collect::<Vec<_>>()
                .join(", ");
            let mut out = String::from("\n");
            out.push_str(&javadoc("    ", &op_doc(op), &all_tags));
            let _ = write!(
                out,
                "    public static {stream}<{answer}> {fname}({args}) {{\n        return client.stream({build}, {answer}.class);\n    }}\n"
            );
            return out;
        }
        let mut sync_tags = all_tags.clone();
        sync_tags.push("@return the answer, typed, and the raw one".into());
        let mut async_tags = all_tags;
        async_tags.push("@return the answer, when it arrives".into());
        imports.add("java.util.concurrent.CompletableFuture");
        let args = std::iter::once(format!("{client} client"))
            .chain(decl)
            .collect::<Vec<_>>()
            .join(", ");
        let mut out = String::new();
        out.push('\n');
        out.push_str(&javadoc("    ", &op_doc(op), &sync_tags));
        let _ = write!(
            out,
            "    public static {response}<{answer}> {fname}({args}) {{\n        return client.request({build}, {answer}.class);\n    }}\n"
        );
        out.push('\n');
        out.push_str(&javadoc(
            "    ",
            &format!("{} On the client's executor.", op_doc(op)),
            &async_tags,
        ));
        let _ = write!(
            out,
            "    public static CompletableFuture<{response}<{answer}>> {fname}Async({args}) {{\n        return client.requestAsync({build}, {answer}.class);\n    }}\n"
        );
        out
    }

    /// The type the answer is read as.
    fn answer(&self, op: &Op, imports: &mut Imports) -> String {
        if let Some(r) = &op.response {
            self.ty(&Type::Ref { name: r.clone() }, true, imports)
        } else {
            imports.add("com.fasterxml.jackson.databind.JsonNode");
            "JsonNode".into()
        }
    }

    /// An operation's arguments after the client: declarations, their `@param` tags, and
    /// the names to pass on.
    fn arguments(&self, op: &Op, imports: &mut Imports) -> (Vec<String>, Vec<String>, Vec<String>) {
        let naming = JavaNaming;
        let mut decl = Vec::new();
        let mut tags = Vec::new();
        let mut call = Vec::new();
        for p in &op.path_params {
            let name = naming.field_name(&p.name);
            decl.push(format!("String {name}"));
            tags.push(format!(
                "@param {name} the <code>{}</code> path parameter",
                jdoc(&p.name)
            ));
            call.push(name);
        }
        if let Some(t) = &op.params_type {
            decl.push(format!("{t} params"));
            tags.push("@param params the query parameters".to_owned());
            call.push("params".into());
        }
        if let Some(b) = &op.body {
            let ty = self.ty(&Type::Ref { name: b.clone() }, true, imports);
            decl.push(format!("{ty} body"));
            tags.push("@param body the request body".into());
            call.push("body".into());
        }
        (decl, tags, call)
    }

    fn operations(&self, all: &[&Op], surface: &context::Surface) -> String {
        let mut imports = Imports::default();
        imports.add(format!("{}.codegen.Codegen", self.runtime));
        let mut methods = String::new();
        for op in all {
            methods.push_str(&self.operation(op, &mut imports));
        }
        let mut body = javadoc(
            "",
            &format!(
                "Every operation of this surface (API {}) as a static method over the runtime's one request path; the profile classes call these.",
                jdoc(&surface.api_version)
            ),
            &[],
        );
        body.push_str("public final class Operations {\n");
        body.push_str("\n    /** A surface generated for another runtime version does not compile here: run <code>iohr sdk generate</code> again. */\n    public static final int SURFACE = Codegen.V1;\n\n    private Operations() {}\n");
        body.push_str(&methods);
        body.push_str("}\n");
        self.file(&imports, &body)
    }

    /// One profile's class and its handle classes, as (type name, file).
    fn profile(&self, surface: &context::Surface, p: &context::Profile) -> Vec<(String, String)> {
        let naming = JavaNaming;
        let class = naming.type_name(&p.name);
        let handles: Vec<(&context::Handle, Vec<&Op>)> = surface
            .handles
            .iter()
            .map(|h| {
                (
                    h,
                    h.ops
                        .iter()
                        .filter(|o| o.profiles.contains(&p.name))
                        .collect::<Vec<_>>(),
                )
            })
            .filter(|(_, ops)| !ops.is_empty())
            .collect();
        let mut files = Vec::new();
        let mut imports = Imports::default();
        imports.add("java.util.Objects");
        let client = self.rt("Client", &mut imports);
        let tags: Vec<&str> = handles.iter().map(|(h, _)| h.tag.as_str()).collect();
        let mut body = profile_head(&class, p, &tags, &client);
        body.push_str("    }\n");
        let env_arg = if p.is_public {
            String::new()
        } else {
            lit(&p.env)
        };
        let _ = write!(
            body,
            "\n{}    public static {class} fromEnv() {{\n        return new {class}({client}.fromEnv({env_arg}));\n    }}\n",
            javadoc(
                "    ",
                "The profile with its credential from the environment.",
                &[
                    "@return the profile".to_owned(),
                    format!(
                        "@throws {}.errors.ConfigException naming the variables to set when no credential is there",
                        self.runtime
                    ),
                ]
            )
        );
        let _ = write!(
            body,
            "\n{}    public {client} client() {{\n        return client;\n    }}\n",
            javadoc(
                "    ",
                "The client the profile calls with.",
                &["@return the client".to_owned()]
            )
        );
        for (h, _) in &handles {
            let _ = write!(
                body,
                "\n{}    public {class}{} {}() {{\n        return {};\n    }}\n",
                javadoc(
                    "    ",
                    &format!("The <code>{}</code> operations.", jdoc(&h.tag)),
                    &["@return the handle".to_owned()]
                ),
                naming.type_name(&h.tag),
                naming.method_name(&h.tag),
                naming.method_name(&h.tag)
            );
        }
        for op in surface.flat.iter().filter(|o| o.profiles.contains(&p.name)) {
            body.push_str(&self.delegate(op, &mut imports));
        }
        body.push_str("}\n");
        files.push((class.clone(), self.file(&imports, &body)));
        for (h, ops) in &handles {
            files.push(self.handle(&class, p, h, ops));
        }
        files
    }

    /// One handle class: the operations of one tag a profile may call.
    fn handle(
        &self,
        class: &str,
        p: &context::Profile,
        h: &context::Handle,
        ops: &[&Op],
    ) -> (String, String) {
        let naming = JavaNaming;
        let name = format!("{class}{}", naming.type_name(&h.tag));
        let mut imports = Imports::default();
        imports.add("java.util.Objects");
        let client = self.rt("Client", &mut imports);
        let mut body = javadoc(
            "",
            &format!(
                "The <code>{}</code> operations profile <code>{}</code> may call.",
                jdoc(&h.tag),
                jdoc(&p.name)
            ),
            &[],
        );
        let _ = write!(
            body,
            "public final class {name} {{\n\n    private final {client} client;\n\n    {name}({client} client) {{\n        this.client = Objects.requireNonNull(client, \"client\");\n    }}\n"
        );
        for op in ops {
            body.push_str(&self.delegate(op, &mut imports));
        }
        body.push_str("}\n");
        let text = self.file(&imports, &body);
        (name, text)
    }

    /// A profile's method: the operation, on the profile's client; an overload without the
    /// query parameters when none is required.
    fn delegate(&self, op: &Op, imports: &mut Imports) -> String {
        let naming = JavaNaming;
        let response = self.rt("Response", imports);
        let answer = self.answer(op, imports);
        let (decl, tags, call) = self.arguments(op, imports);
        let name = naming.method_name(&op.name);
        let fname = function_name(op);
        imports.add("java.util.concurrent.CompletableFuture");
        let mut out = String::new();
        let mut variants = vec![(decl.clone(), tags.clone(), call.clone())];
        if let Some(t) = &op.params_type
            && !op.query.iter().any(|q| q.required)
        {
            let keep = |v: &Vec<String>, skip: &str| -> Vec<String> {
                v.iter().filter(|x| !x.contains(skip)).cloned().collect()
            };
            let call2: Vec<String> = call
                .iter()
                .map(|c| {
                    if c == "params" {
                        format!("{t}.builder().build()")
                    } else {
                        c.clone()
                    }
                })
                .collect();
            variants.push((
                keep(&decl, &format!("{t} params")),
                keep(&tags, "@param params"),
                call2,
            ));
        }
        for (decl, tags, call) in variants {
            let args = decl.join(", ");
            let pass = std::iter::once("client".to_owned())
                .chain(call)
                .collect::<Vec<_>>()
                .join(", ");
            if op.stream {
                let stream = self.rt("EventStream", imports);
                let mut stream_tags = tags;
                stream_tags
                    .push("@return the events, one model each; close it to stop early".into());
                out.push('\n');
                out.push_str(&javadoc("    ", &op_doc(op), &stream_tags));
                let _ = write!(
                    out,
                    "    public {stream}<{answer}> {name}({args}) {{\n        return Operations.{fname}({pass});\n    }}\n"
                );
                continue;
            }
            let mut sync_tags = tags.clone();
            sync_tags.push("@return the answer, typed, and the raw one".into());
            let mut async_tags = tags;
            async_tags.push("@return the answer, when it arrives".into());
            out.push('\n');
            out.push_str(&javadoc("    ", &op_doc(op), &sync_tags));
            let _ = write!(
                out,
                "    public {response}<{answer}> {name}({args}) {{\n        return Operations.{fname}({pass});\n    }}\n"
            );
            out.push('\n');
            out.push_str(&javadoc(
                "    ",
                &format!("{} On the client's executor.", op_doc(op)),
                &async_tags,
            ));
            let _ = write!(
                out,
                "    public CompletableFuture<{response}<{answer}>> {name}Async({args}) {{\n        return Operations.{fname}Async({pass});\n    }}\n"
            );
        }
        if op.body.is_none()
            && !op.stream
            && let Some(paging) = context::paging(op, &self.models)
        {
            out.push_str(&self.iterator(op, &paging, imports));
        }
        out
    }

    /// `all<Op>`: every item of a paged list as a lazy [`Iterable`] (design.md §9); an
    /// overload without the query parameters when none is required.
    fn iterator(&self, op: &Op, paging: &context::Paging, imports: &mut Imports) -> String {
        let naming = JavaNaming;
        let pages = self.rt("Pages", imports);
        imports.add(format!("{}.codegen.Codegen", self.runtime));
        let item = self.ty(&paging.item, true, imports);
        let Some(params_type) = &op.params_type else {
            return String::new();
        };
        let name = format!("all{}", naming.type_name(&op.name));
        let fname = function_name(op);
        let path: Vec<String> = op
            .path_params
            .iter()
            .map(|p| naming.field_name(&p.name))
            .collect();
        // The record again with the token replaced once there is one: the caller's own
        // token starts the walk.
        let components: Vec<String> = op
            .query
            .iter()
            .map(|q| {
                let f = naming.field_name(&q.name);
                if q.name == paging.token_param {
                    format!("token != null ? token : params.{f}()")
                } else {
                    format!("params.{f}()")
                }
            })
            .collect();
        let pass: Vec<String> = std::iter::once("client".to_owned())
            .chain(path.iter().cloned())
            .chain(std::iter::once(format!(
                "new {params_type}({})",
                components.join(", ")
            )))
            .collect();
        let list = naming.field_name(&paging.list_field);
        let next = naming.field_name(&paging.next_field);
        let main = format!(
            "Every item <code>{}</code> answers, page after page, following <code>{}</code> until the last page; see {{@link {pages}}}.",
            jdoc(&op.line),
            jdoc(&paging.next_field)
        );
        let mut decl: Vec<String> = path.iter().map(|p| format!("String {p}")).collect();
        let mut tags: Vec<String> = path
            .iter()
            .map(|p| format!("@param {p} the <code>{p}</code> path parameter"))
            .collect();
        decl.push(format!("{params_type} params"));
        tags.push("@param params the query parameters; the page token is set for each page".into());
        tags.push("@return the walk".into());
        let mut out = String::from("\n");
        out.push_str(&javadoc("    ", &main, &tags));
        let _ = write!(
            out,
            "    public {pages}<{item}> {name}({}) {{\n        return Codegen.pages(token -> {{\n            var page = Operations.{fname}({}).value();\n            return new {pages}.Page<>(page.{list}(), page.{next}());\n        }});\n    }}\n",
            decl.join(", "),
            pass.join(", ")
        );
        if !op.query.iter().any(|q| q.required) {
            let short: Vec<String> = path.iter().map(|p| format!("String {p}")).collect();
            let args: Vec<String> = path
                .iter()
                .cloned()
                .chain(std::iter::once(format!("{params_type}.builder().build()")))
                .collect();
            let mut short_tags: Vec<String> = path
                .iter()
                .map(|p| format!("@param {p} the <code>{p}</code> path parameter"))
                .collect();
            short_tags.push("@return the walk".into());
            out.push('\n');
            out.push_str(&javadoc("    ", &main, &short_tags));
            let _ = write!(
                out,
                "    public {pages}<{item}> {name}({}) {{\n        return {name}({});\n    }}\n",
                short.join(", "),
                args.join(", ")
            );
        }
        out
    }

    fn package_info(&self, surface: &context::Surface) -> String {
        let profiles: Vec<String> = surface
            .profiles
            .iter()
            .map(|p| format!("<code>{}</code>", jdoc(&p.name)))
            .collect();
        format!(
            "{}/**\n * The InOrbit API surface for {} (API {}): the models and the operations these profiles may\n * call, on the <code>{}</code> runtime. Generated by <code>iohr sdk generate</code>; regenerate it, never\n * edit it. <code>iohr sdk check</code> says when the API's cut has moved.\n */\npackage {};\n",
            self.header,
            profiles.join(", "),
            jdoc(&surface.api_version),
            self.runtime,
            self.package
        )
    }
}

/// A profile class up to the end of its constructor: its Javadoc, its fields, and the
/// constructor that builds the handles for `tags`.
fn profile_head(class: &str, p: &context::Profile, tags: &[&str], client: &str) -> String {
    let naming = JavaNaming;
    let env_doc = if p.is_public {
        "<code>fromEnv()</code> reads <code>INORBIT_TOKEN</code>, or <code>INORBIT_KEY_ID</code>, <code>INORBIT_KEY_SECRET</code> and <code>INORBIT_SCOPES</code>.".to_owned()
    } else {
        format!(
            "<code>fromEnv()</code> reads <code>INORBIT_{e}_TOKEN</code>, or <code>INORBIT_{e}_KEY_ID</code>, <code>INORBIT_{e}_KEY_SECRET</code> and <code>INORBIT_{e}_SCOPES</code>, and nothing else.",
            e = p.env
        )
    };
    let mut body = javadoc(
        "",
        &format!(
            "Profile <code>{}</code>: the operations its cut holds. {env_doc}",
            jdoc(&p.name)
        ),
        &[],
    );
    let _ = writeln!(body, "public final class {class} {{");
    let _ = writeln!(
        body,
        "\n    /** The profile's name. */\n    public static final String PROFILE = {};\n\n    private final {client} client;",
        lit(&p.name)
    );
    for tag in tags {
        let _ = writeln!(
            body,
            "    private final {class}{} {};",
            naming.type_name(tag),
            naming.method_name(tag)
        );
    }
    let _ = write!(
        body,
        "\n{}    public {class}({client} client) {{\n        this.client = Objects.requireNonNull(client, \"client\");\n",
        javadoc(
            "    ",
            "The profile on a client built for its credential.",
            &["@param client the client".to_owned()]
        )
    );
    for tag in tags {
        let _ = writeln!(
            body,
            "        this.{} = new {class}{}(client);",
            naming.method_name(tag),
            naming.type_name(tag)
        );
    }
    body
}

/// Where a 64-bit integer sits in a field.
enum Int64At {
    None,
    Value,
    Contents,
}

/// The static method an operation becomes in `Operations`: named after its marker, which
/// is unique.
fn function_name(op: &Op) -> String {
    JavaNaming.escape(op.marker.to_lower_camel_case())
}

fn op_doc(op: &Op) -> String {
    let mut text = format!("<code>{}</code>", jdoc(&op.line));
    if !op.scopes.is_empty() {
        let list: Vec<String> = op
            .scopes
            .iter()
            .map(|s| format!("<code>{}</code>", jdoc(s)))
            .collect();
        let _ = write!(
            text,
            "; needs scope{} {}",
            if op.scopes.len() > 1 { "s" } else { "" },
            list.join(", ")
        );
    }
    text.push('.');
    if !op.doc.is_empty() {
        let _ = write!(text, " {}", jdoc(&op.doc));
    }
    text
}

/// A nested builder for a record with `fields` (name, type).
fn builder_class(record: &str, fields: &[(String, String)]) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "\n    /**\n     * A builder.\n     *\n     * @return a builder with every field unset\n     */\n    public static Builder builder() {{\n        return new Builder();\n    }}\n\n    /** Builds a <code>{record}</code>. */\n    public static final class Builder {{\n"
    );
    for (name, ty) in fields {
        let _ = writeln!(out, "        private {ty} {name};");
    }
    out.push_str("\n        private Builder() {}\n");
    for (name, ty) in fields {
        let _ = write!(
            out,
            "\n        /**\n         * Sets <code>{name}</code>.\n         *\n         * @param {name} the value\n         * @return this builder\n         */\n        public Builder {name}({ty} {name}) {{\n            this.{name} = {name};\n            return this;\n        }}\n"
        );
    }
    let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
    let _ = write!(
        out,
        "\n        /**\n         * The value.\n         *\n         * @return the <code>{record}</code>\n         */\n        public {record} build() {{\n            return new {record}({});\n        }}\n    }}\n",
        names.join(", ")
    );
    out
}

#[cfg(test)]
mod tests {
    use super::{JavaNaming, check_package, jdoc};
    use crate::context::Naming as _;

    #[test]
    fn javadoc_text_is_escaped() {
        assert_eq!(
            jdoc("a `x<y>` & b @c */"),
            "a <code>x&lt;y&gt;</code> &amp; b &#64;c &#42;/"
        );
    }

    #[test]
    fn names_avoid_keywords_and_object_methods() {
        assert_eq!(JavaNaming.field_name("class"), "class_");
        assert_eq!(JavaNaming.field_name("hash_code"), "hashCode_");
        assert_eq!(JavaNaming.field_name("org_id"), "orgId");
        assert_eq!(JavaNaming.method_name("get_usage"), "getUsage");
    }

    #[test]
    fn package_names_are_checked() {
        assert!(check_package("com.example.iohr").is_ok());
        assert!(check_package("hr.inorbit.sdk.generated").is_ok());
        assert!(check_package("com.class.x").is_err());
        assert!(check_package("1x").is_err());
        assert!(check_package("").is_err());
    }
}
