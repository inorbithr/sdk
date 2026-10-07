//! The Swift target: models as `Codable` structs, one marker protocol per operation,
//! and one profile type per profile adopting the markers its cut holds. Operations are
//! methods in constrained extensions of the runtime's `Client<P>` (flat ones) or of a
//! handle per tag (`extension RadarHandle where P: AllowsListDigests`), so a call the
//! profile may not make does not compile, as in Rust and C#.
//!
//! Every model spells out its `Codable` conformance: properties are `lowerCamel`, keys
//! are the wire names, an unset optional is left out of the body (design.md section 12,
//! "Wire optionality"), and a 64-bit integer travels as a decimal string through the
//! runtime's `WireInt64` and is an `Int64` everywhere else. A string enum keeps a value
//! this version does not know, and an inline enum is a `String`.

mod example;
pub(crate) use example::example;

use std::fmt::Write as _;

use heck::{ToLowerCamelCase as _, ToUpperCamelCase as _};
use iohr_openapi::{Api, Method};

use crate::context::{self, Naming, Op, Segment, Surface};
use crate::files::Files;
use crate::ir::{self, Field, Model, Shape, Type};
use crate::language::{Language, Options};
use crate::target::{RenderError, Target};

/// The Swift target.
#[derive(Debug, Clone, Copy, Default)]
pub struct SwiftTarget;

/// How Swift spells names: `UpperCamel` types, `lowerCamel` methods, properties and
/// parameters, a reserved word escaped with backticks.
struct SwiftNaming;

impl Naming for SwiftNaming {
    const RESERVED: &'static [&'static str] = &[
        "Any",
        "Protocol",
        "Self",
        "Type",
        "as",
        "associatedtype",
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "continue",
        "default",
        "defer",
        "deinit",
        "do",
        "else",
        "enum",
        "extension",
        "fallthrough",
        "false",
        "fileprivate",
        "for",
        "func",
        "guard",
        "if",
        "import",
        "in",
        "init",
        "inout",
        "internal",
        "is",
        "let",
        "nil",
        "open",
        "operator",
        "private",
        "precedencegroup",
        "protocol",
        "public",
        "repeat",
        "rethrows",
        "return",
        "self",
        "static",
        "struct",
        "subscript",
        "super",
        "switch",
        "throw",
        "throws",
        "true",
        "try",
        "typealias",
        "var",
        "where",
        "while",
    ];

    fn escape(&self, ident: String) -> String {
        if Self::RESERVED.contains(&ident.as_str()) {
            format!("`{ident}`")
        } else {
            ident
        }
    }

    fn method_name(&self, snake: &str) -> String {
        self.escape(snake.to_lower_camel_case())
    }

    fn field_name(&self, wire: &str) -> String {
        let name = wire.to_lower_camel_case();
        // An identifier cannot start with a digit (`24h_count`).
        let name = if name.starts_with(|c: char| c.is_ascii_digit()) {
            format!("_{name}")
        } else {
            name
        };
        self.escape(name)
    }
}

impl Target for SwiftTarget {
    const LANG: Language = Language::Swift;

    fn render(&self, api: &Api, options: &Options) -> Result<Files, RenderError> {
        let ctx = Ctx {
            rt: options.runtime_for(Language::Swift),
            in_package: options.in_package,
        };
        let surface = context::surface(api);
        let models = context::models(api);
        let mut files = Files::new();
        for note in &surface.notes {
            files.note(note.clone());
        }
        let head = header(&surface.header, &ctx);
        files.insert(
            "Models.swift",
            format!("{head}{}", render_models(&models, &ctx)),
        );
        files.insert(
            "Operations.swift",
            format!("{head}{}", render_operations(&surface, &models, &ctx)),
        );
        files.insert(
            "Profiles.swift",
            format!("{head}{}", render_profiles(&surface, &ctx)),
        );
        Ok(files)
    }
}

/// What every rendering function needs: the runtime module and whether the surface is the
/// runtime's own (`--in-package`), which imports nothing and extends the runtime's
/// `Public` profile.
struct Ctx {
    rt: String,
    in_package: bool,
}

impl Ctx {
    /// A runtime type, qualified by its module so a model of the same name cannot shadow it.
    fn rt(&self, name: &str) -> String {
        format!("{}.{name}", self.rt)
    }
}

fn header(line: &str, ctx: &Ctx) -> String {
    let mut out = format!("// {line}\n// swiftlint:disable all\n\n");
    out.push_str("import Foundation\n");
    if !ctx.in_package {
        let _ = writeln!(out, "import {}", ctx.rt);
    }
    out
}

/// A doc comment, or nothing.
fn doc(text: &str, indent: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    format!("{indent}/// {}\n", text.replace('\n', " "))
}

/// A Swift string literal.
fn lit(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The native type of `ty`.
fn swift_type(ty: &Type, ctx: &Ctx) -> String {
    match ty {
        Type::Ref { name } if ir::RUNTIME_TYPES.contains(&name.as_str()) => ctx.rt(name),
        Type::Ref { name } => name.clone(),
        Type::Int64 | Type::Integer { bits: 64 } => "Int64".into(),
        Type::Integer { .. } => "Int32".into(),
        Type::Number => "Double".into(),
        Type::String | Type::Enum { .. } => "String".into(),
        Type::Bool => "Bool".into(),
        Type::Array { item } => format!("[{}]", swift_type(item, ctx)),
        Type::Map { value } => format!("[String: {}]", swift_type(value, ctx)),
        _ => ctx.rt("JSONValue"),
    }
}

/// Whether `ty` holds a decimal-string 64-bit integer, which travels as `WireInt64`.
fn has_int64(ty: &Type) -> bool {
    match ty {
        Type::Int64 => true,
        Type::Array { item } => has_int64(item),
        Type::Map { value } => has_int64(value),
        _ => false,
    }
}

/// The type `ty` is decoded as and encoded from: the native type with every decimal-string
/// 64-bit integer as `WireInt64`.
fn wire_type(ty: &Type, ctx: &Ctx) -> String {
    match ty {
        Type::Int64 => ctx.rt("WireInt64"),
        Type::Array { item } => format!("[{}]", wire_type(item, ctx)),
        Type::Map { value } => format!("[String: {}]", wire_type(value, ctx)),
        other => swift_type(other, ctx),
    }
}

/// `expr` (of the wire type) as the native type.
fn from_wire(expr: &str, ty: &Type) -> String {
    match ty {
        Type::Int64 => format!("{expr}.value"),
        Type::Array { item } if has_int64(item) => {
            format!("{expr}.map {{ {} }}", from_wire("$0", item))
        }
        Type::Map { value } if has_int64(value) => {
            format!("{expr}.mapValues {{ {} }}", from_wire("$0", value))
        }
        _ => expr.to_owned(),
    }
}

/// `expr` (of the native type) as the wire type.
fn to_wire(expr: &str, ty: &Type, ctx: &Ctx) -> String {
    match ty {
        Type::Int64 => format!("{}({expr})", ctx.rt("WireInt64")),
        Type::Array { item } if has_int64(item) => {
            format!("{expr}.map {{ {} }}", to_wire("$0", item, ctx))
        }
        Type::Map { value } if has_int64(value) => {
            format!("{expr}.mapValues {{ {} }}", to_wire("$0", value, ctx))
        }
        _ => expr.to_owned(),
    }
}

/// Whether a field reads as optional: not required, or required but nullable.
fn optional(f: &Field) -> bool {
    !f.required || f.nullable
}

fn render_models(models: &[Model], ctx: &Ctx) -> String {
    let mut out = String::new();
    for m in models.iter().filter(|m| !m.runtime) {
        out.push('\n');
        out.push_str(&doc(&m.doc, ""));
        match &m.shape {
            Shape::Object { fields } => out.push_str(&object(&m.name, fields, ctx)),
            Shape::Enum { values } => out.push_str(&string_enum(&m.name, values)),
            Shape::Union {
                variants,
                discriminator,
            } => out.push_str(&union(&m.name, variants, discriminator.as_deref(), ctx)),
            Shape::Alias { ty } => {
                let _ = writeln!(out, "public typealias {} = {}", m.name, swift_type(ty, ctx));
            }
        }
    }
    out
}

fn object(name: &str, fields: &[Field], ctx: &Ctx) -> String {
    let naming = SwiftNaming;
    if fields.is_empty() {
        return format!(
            "public struct {name}: Codable, Sendable {{\n    /// An empty value.\n    public init() {{}}\n}}\n"
        );
    }
    let mut out = format!("public struct {name}: Codable, Sendable {{\n");
    for f in fields {
        let ty = swift_type(&f.ty, ctx);
        let q = if optional(f) { "?" } else { "" };
        let _ = writeln!(
            out,
            "{}    public var {}: {ty}{q}",
            doc(&f.doc, "    "),
            naming.field_name(&f.name)
        );
    }
    // The memberwise initialiser: what is required comes without a default.
    let params: Vec<String> = fields
        .iter()
        .map(|f| {
            let ty = swift_type(&f.ty, ctx);
            if optional(f) {
                format!("{}: {ty}? = nil", naming.field_name(&f.name))
            } else {
                format!("{}: {ty}", naming.field_name(&f.name))
            }
        })
        .collect();
    let _ = writeln!(
        out,
        "\n    /// A value with every field given; what is left `nil` is left out of a request.\n    public init(\n        {}\n    ) {{",
        params.join(",\n        ")
    );
    for f in fields {
        let n = naming.field_name(&f.name);
        let _ = writeln!(out, "        self.{n} = {n}");
    }
    out.push_str("    }\n\n    enum CodingKeys: String, CodingKey {\n");
    for f in fields {
        let _ = writeln!(
            out,
            "        case {} = {}",
            naming.field_name(&f.name),
            lit(&f.name)
        );
    }
    out.push_str("    }\n\n    public init(from decoder: any Decoder) throws {\n        let c = try decoder.container(keyedBy: CodingKeys.self)\n");
    for f in fields {
        let n = naming.field_name(&f.name);
        let w = wire_type(&f.ty, ctx);
        if optional(f) {
            let read = format!("try c.decodeIfPresent({w}.self, forKey: .{n})");
            if has_int64(&f.ty) {
                let _ = writeln!(
                    out,
                    "        self.{n} = {read}.map {{ {} }}",
                    from_wire("$0", &f.ty)
                );
            } else {
                let _ = writeln!(out, "        self.{n} = {read}");
            }
        } else {
            let read = format!("try c.decode({w}.self, forKey: .{n})");
            let _ = writeln!(out, "        self.{n} = {}", from_wire(&read, &f.ty));
        }
    }
    out.push_str("    }\n\n    public func encode(to encoder: any Encoder) throws {\n        var c = encoder.container(keyedBy: CodingKeys.self)\n");
    for f in fields {
        let n = naming.field_name(&f.name);
        if optional(f) {
            if has_int64(&f.ty) {
                let _ = writeln!(
                    out,
                    "        try c.encodeIfPresent(self.{n}.map {{ {} }}, forKey: .{n})",
                    to_wire("$0", &f.ty, ctx)
                );
            } else {
                let _ = writeln!(out, "        try c.encodeIfPresent(self.{n}, forKey: .{n})");
            }
        } else {
            let _ = writeln!(
                out,
                "        try c.encode({}, forKey: .{n})",
                to_wire(&format!("self.{n}"), &f.ty, ctx)
            );
        }
    }
    out.push_str("    }\n}\n");
    out
}

/// A string enum that keeps a value this version does not know.
fn string_enum(name: &str, values: &[String]) -> String {
    let naming = SwiftNaming;
    let mut out = format!(
        "public struct {name}: RawRepresentable, Codable, Hashable, Sendable, ExpressibleByStringLiteral {{\n    /// The value as the API spells it.\n    public let rawValue: String\n\n    /// A value, known to this version or not.\n    public init(rawValue: String) {{\n        self.rawValue = rawValue\n    }}\n\n    /// A value from a literal.\n    public init(stringLiteral value: String) {{\n        self.rawValue = value\n    }}\n\n    public init(from decoder: any Decoder) throws {{\n        self.rawValue = try decoder.singleValueContainer().decode(String.self)\n    }}\n\n    public func encode(to encoder: any Encoder) throws {{\n        var c = encoder.singleValueContainer()\n        try c.encode(self.rawValue)\n    }}\n"
    );
    for v in values {
        let _ = writeln!(
            out,
            "\n    /// `{v}`.\n    public static let {} = {name}(rawValue: {})",
            naming.field_name(v),
            lit(v)
        );
    }
    out.push_str("}\n");
    out
}

/// One of several models: by the discriminator's value when the schema names one (the
/// variant's name, or its name in `snake_case`), otherwise the first variant that
/// decodes; a value no variant reads is kept as `unknown`.
fn union(name: &str, variants: &[Type], discriminator: Option<&str>, ctx: &Ctx) -> String {
    let naming = SwiftNaming;
    let cases: Vec<(String, String, String)> = variants
        .iter()
        .map(|v| {
            let ty = swift_type(v, ctx);
            let raw = match v {
                Type::Ref { name } => name.clone(),
                _ => ty.clone(),
            };
            (naming.field_name(&raw), ty, raw)
        })
        .collect();
    let mut out = format!("public enum {name}: Codable, Sendable {{\n");
    for (case, ty, _) in &cases {
        let _ = writeln!(out, "    case {case}({ty})");
    }
    let json = ctx.rt("JSONValue");
    let _ = writeln!(
        out,
        "    /// A value no variant of this version reads, kept as it came.\n    case unknown({json})\n\n    public init(from decoder: any Decoder) throws {{"
    );
    if let Some(d) = discriminator {
        let _ = writeln!(
            out,
            "        let raw = try {json}(from: decoder)\n        let tag = raw[{}]?.stringValue ?? \"\"\n        switch tag {{",
            lit(d)
        );
        for (case, ty, raw) in &cases {
            let snake = heck::ToSnakeCase::to_snake_case(raw.as_str());
            let _ = writeln!(
                out,
                "        case {}, {}: self = .{case}(try {ty}(from: decoder))",
                lit(raw),
                lit(&snake)
            );
        }
        out.push_str("        default: self = .unknown(raw)\n        }\n    }\n");
    } else {
        for (case, ty, _) in &cases {
            let _ = writeln!(
                out,
                "        if let v = try? {ty}(from: decoder) {{\n            self = .{case}(v)\n            return\n        }}"
            );
        }
        let _ = writeln!(
            out,
            "        self = .unknown(try {json}(from: decoder))\n    }}"
        );
    }
    out.push_str(
        "\n    public func encode(to encoder: any Encoder) throws {\n        switch self {\n",
    );
    for (case, _, _) in &cases {
        let _ = writeln!(
            out,
            "        case .{case}(let v): try v.encode(to: encoder)"
        );
    }
    out.push_str("        case .unknown(let v): try v.encode(to: encoder)\n        }\n    }\n}\n");
    out
}

/// The marker protocol of an operation.
fn marker(op_marker: &str) -> String {
    format!("Allows{op_marker}")
}

/// The protocol of a profile whose cut holds an operation of `tag`.
fn area(tag: &str) -> String {
    format!("Allows{}Area", tag.to_upper_camel_case())
}

fn handle_type(tag: &str) -> String {
    format!("{}Handle", tag.to_upper_camel_case())
}

fn method(m: Method) -> &'static str {
    match m {
        Method::Get => ".get",
        Method::Post => ".post",
        Method::Put => ".put",
        Method::Patch => ".patch",
        Method::Delete => ".delete",
        Method::Head => ".head",
    }
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

fn all_ops(surface: &Surface) -> Vec<&Op> {
    let mut ops: Vec<&Op> = surface
        .handles
        .iter()
        .flat_map(|h| h.ops.iter())
        .chain(surface.flat.iter())
        .collect();
    ops.sort_by(|a, b| a.line.cmp(&b.line));
    ops
}

fn render_operations(surface: &Surface, models: &[Model], ctx: &Ctx) -> String {
    let mut out = format!(
        "\n/// A surface generated for another runtime contract fails to compile here: run `iohr sdk generate` again.\nprivate enum SurfaceContract {{\n    static let version: {}.Type = {}.self\n}}\n",
        ctx.rt("Codegen.Version1"),
        ctx.rt("Codegen.Version1")
    );
    for m in &surface.markers {
        let _ = write!(
            out,
            "\n/// A profile whose cut holds `{}`.\npublic protocol {}: {} {{}}\n",
            m.line,
            marker(&m.name),
            ctx.rt("Profile")
        );
    }
    for h in &surface.handles {
        let _ = write!(
            out,
            "\n/// A profile whose cut holds an operation of `{}`.\npublic protocol {}: {} {{}}\n",
            h.tag,
            area(&h.tag),
            ctx.rt("Profile")
        );
    }
    for op in all_ops(surface) {
        if let Some(t) = &op.params_type {
            out.push_str(&params_struct(t, op, ctx));
        }
    }
    // The client's own extensions: a handle per tag, and the flat operations.
    for h in &surface.handles {
        let handle = handle_type(&h.tag);
        let _ = write!(
            out,
            "\n/// The `{tag}` operations, on a profile's client.\npublic struct {handle}<P: {profile}>: Sendable {{\n    let client: {client}<P>\n}}\n\nextension {client} where P: {area} {{\n    /// The `{tag}` operations of the profile's cut.\n    public var {name}: {handle}<P> {{\n        {handle}(client: self)\n    }}\n}}\n",
            tag = h.tag,
            profile = ctx.rt("Profile"),
            client = ctx.rt("Client"),
            area = area(&h.tag),
            name = SwiftNaming.method_name(&h.tag),
        );
    }
    for op in &surface.flat {
        let _ = write!(
            out,
            "\nextension {} where P: {} {{\n{}{}}}\n",
            ctx.rt("Client"),
            marker(&op.marker),
            operation(op, "self", ctx),
            iterator(op, models, ctx)
        );
    }
    for h in &surface.handles {
        for op in &h.ops {
            let _ = write!(
                out,
                "\nextension {} where P: {} {{\n{}{}}}\n",
                handle_type(&h.tag),
                marker(&op.marker),
                operation(op, "client", ctx),
                iterator(op, models, ctx)
            );
        }
    }
    out
}

fn params_struct(type_name: &str, op: &Op, ctx: &Ctx) -> String {
    let naming = SwiftNaming;
    let mut out = format!(
        "\n/// The query parameters of `{}`.\npublic struct {type_name}: Sendable {{\n",
        op.line
    );
    for q in &op.query {
        let ty = swift_type(&q.ty, ctx);
        let q_mark = if q.required { "" } else { "?" };
        let _ = writeln!(
            out,
            "{}    public var {}: {ty}{q_mark}",
            doc(&q.doc, "    "),
            naming.field_name(&q.name)
        );
    }
    let params: Vec<String> = op
        .query
        .iter()
        .map(|q| {
            let ty = swift_type(&q.ty, ctx);
            if q.required {
                format!("{}: {ty}", naming.field_name(&q.name))
            } else {
                format!("{}: {ty}? = nil", naming.field_name(&q.name))
            }
        })
        .collect();
    let _ = writeln!(
        out,
        "\n    /// The parameters; what is left `nil` is not sent.\n    public init(\n        {}\n    ) {{",
        params.join(",\n        ")
    );
    for q in &op.query {
        let n = naming.field_name(&q.name);
        let _ = writeln!(out, "        self.{n} = {n}");
    }
    out.push_str("    }\n}\n");
    out
}

/// An operation's parameters: the path parameters, the query, the body and the call
/// options, as a declaration.
fn parameters(op: &Op, ctx: &Ctx) -> Vec<String> {
    let naming = SwiftNaming;
    let mut params: Vec<String> = op
        .path_params
        .iter()
        .map(|p| format!("{}: String", naming.field_name(&p.name)))
        .collect();
    if let Some(t) = &op.params_type {
        if op.query.iter().any(|q| q.required) {
            params.push(format!("_ query: {t}"));
        } else {
            params.push(format!("_ query: {t} = {t}()"));
        }
    }
    if let Some(body) = &op.body {
        params.push(format!("body: {body}"));
    }
    params.push(format!("options: {} = .init()", ctx.rt("CallOptions")));
    params
}

/// The statements that build an operation's `Codegen.Operation`.
fn build_operation(op: &Op, ctx: &Ctx) -> String {
    let naming = SwiftNaming;
    let codegen = ctx.rt("Codegen");
    let path: String = op
        .segments
        .iter()
        .map(|s| match s {
            Segment::Literal { text } => text.replace('\\', "\\\\").replace('"', "\\\""),
            Segment::Param { name } => {
                format!("\\({codegen}.pathSegment({}))", naming.field_name(name))
            }
        })
        .collect();
    let scopes: Vec<String> = op.scopes.iter().map(|s| lit(s)).collect();
    let mut out = format!(
        "        var op = {codegen}.Operation(\n            name: {},\n            method: {},\n            path: \"{path}\",\n            template: {},\n            scopes: [{}]\n        )\n",
        lit(&op.hook_name),
        method(op.method),
        lit(&op.path),
        scopes.join(", ")
    );
    if op.idempotent_override {
        out.push_str("        op.idempotent = true\n");
    }
    if op.idempotency_key {
        out.push_str("        op.takesIdempotencyKey = true\n");
    }
    if !op.query.is_empty() {
        let q: Vec<String> = op
            .query
            .iter()
            .map(|q| {
                format!(
                    "({}, {codegen}.queryValues(query.{}))",
                    lit(&q.name),
                    naming.field_name(&q.name)
                )
            })
            .collect();
        let _ = writeln!(
            out,
            "        op.query = [\n            {},\n        ]",
            q.join(",\n            ")
        );
    }
    if op.body.is_some() {
        let _ = writeln!(out, "        op.body = try {codegen}.json(body)");
    }
    if op.stream && !op.rpc.is_empty() {
        let _ = writeln!(out, "        op.rpc = {}", lit(&op.rpc));
    }
    out
}

/// One operation as a method over the runtime's request path; a stream as an
/// `AsyncThrowingStream` of its events (design.md section 7).
fn operation(op: &Op, client: &str, ctx: &Ctx) -> String {
    let naming = SwiftNaming;
    let response = op.response.clone().unwrap_or_else(|| ctx.rt("JSONValue"));
    let params = parameters(op, ctx).join(", ");
    let name = naming.method_name(&op.name);
    if op.stream {
        // Building the operation cannot fail without a body; a stream method does not throw,
        // the stream does (on its first step).
        let build = build_operation(op, ctx);
        let build = if build.contains("op.") {
            build
        } else {
            build.replace("var op", "let op")
        };
        return format!(
            "{}    /// A stream: each event as it happens, until the server ends it; it opens on the first step of `for try await`.\n    public func {name}({params}) -> AsyncThrowingStream<{response}, any Error> {{\n{build}        return {client}.stream(op, as: {response}.self, options: options)\n    }}\n",
            op_doc(op, "    "),
        );
    }
    let build = build_operation(op, ctx);
    let build = if build.contains("op.") {
        build
    } else {
        build.replace("var op", "let op")
    };
    format!(
        "{}    @discardableResult\n    public func {name}({params}) async throws -> {}<{response}> {{\n{build}        return try await {client}.request(op, as: {response}.self, options: options)\n    }}\n",
        op_doc(op, "    "),
        ctx.rt("Response"),
    )
}

/// `all<Op>`: every item of a paged list as an `AsyncThrowingStream` (design.md §9), next to
/// the operation it walks; empty when the operation does not page.
fn iterator(op: &Op, models: &[Model], ctx: &Ctx) -> String {
    let naming = SwiftNaming;
    if op.body.is_some() || op.stream {
        return String::new();
    }
    let (Some(paging), Some(_)) = (context::paging(op, models), op.params_type.as_deref()) else {
        return String::new();
    };
    let item = swift_type(&paging.item, ctx);
    let params = parameters(op, ctx).join(", ");
    let mut args: Vec<String> = op
        .path_params
        .iter()
        .map(|p| {
            let n = naming.field_name(&p.name);
            format!("{}: {n}", n.trim_matches('`'))
        })
        .collect();
    args.push("page".into());
    args.push("options: options".into());
    let token = naming.field_name(&paging.token_param);
    let list = naming.field_name(&paging.list_field);
    let next = naming.field_name(&paging.next_field);
    let list_read = if models
        .iter()
        .find(|m| Some(&m.name) == op.response.as_ref())
        .and_then(|m| match &m.shape {
            Shape::Object { fields } => fields.iter().find(|f| f.name == paging.list_field),
            _ => None,
        })
        .is_some_and(optional)
    {
        format!("value.{list} ?? []")
    } else {
        format!("value.{list}")
    };
    let next_read = if models
        .iter()
        .find(|m| Some(&m.name) == op.response.as_ref())
        .and_then(|m| match &m.shape {
            Shape::Object { fields } => fields.iter().find(|f| f.name == paging.next_field),
            _ => None,
        })
        .is_some_and(optional)
    {
        format!("value.{next} ?? \"\"")
    } else {
        format!("value.{next}")
    };
    format!(
        "\n    /// Every item `{}` answers, page after page, following `{}` until the last page; for `for try await`. It throws the first error, and fetches nothing more once the loop stops.\n    public func all{}({params}) -> AsyncThrowingStream<{item}, any Error> {{\n        {}.pages {{ token in\n            var page = query\n            if let token {{ page.{token} = token }}\n            let value = try await self.{}({}).value\n            return ({list_read}, {next_read})\n        }}\n    }}\n",
        op.line,
        paging.next_field,
        naming.type_name(&op.name),
        ctx.rt("Codegen"),
        naming.method_name(&op.name),
        args.join(", "),
    )
}

fn render_profiles(surface: &Surface, ctx: &Ctx) -> String {
    let mut out = String::new();
    for p in &surface.profiles {
        let adopted = adopted(surface, p);
        if ctx.in_package && p.is_public {
            // The runtime declares `Public`; this surface adds its markers.
            if adopted.is_empty() {
                continue;
            }
            let _ = write!(
                out,
                "\nextension {}:\n    {}\n{{}}\n",
                ctx.rt("Public"),
                adopted.join(",\n    ")
            );
            continue;
        }
        let ty = SwiftNaming.type_name(&p.name);
        let env_doc = if p.is_public {
            "`Client.fromEnv()` reads `INORBIT_TOKEN`, or `INORBIT_KEY_ID`, `INORBIT_KEY_SECRET` and `INORBIT_SCOPES`".to_owned()
        } else {
            format!(
                "`Client.fromEnv()` reads `INORBIT_{e}_TOKEN`, or `INORBIT_{e}_KEY_ID`, `INORBIT_{e}_KEY_SECRET` and `INORBIT_{e}_SCOPES`, and nothing else",
                e = p.env
            )
        };
        let mut conformances = vec![ctx.rt("Profile")];
        conformances.extend(adopted);
        let _ = write!(
            out,
            "\n/// Profile `{}`: the operations its cut holds. {env_doc}.\npublic enum {ty}:\n    {}\n{{\n    public static let name = {}\n    public static let env = {}\n}}\n",
            p.name,
            conformances.join(",\n    "),
            lit(&p.name),
            lit(if p.is_public { "" } else { p.env.as_str() }),
        );
    }
    out
}

/// The protocols profile `p` adopts: the markers and the areas its cut holds.
fn adopted(surface: &Surface, p: &context::Profile) -> Vec<String> {
    let mut list: Vec<String> = surface
        .markers
        .iter()
        .filter(|m| m.profiles.contains(&p.name))
        .map(|m| marker(&m.name))
        .collect();
    list.extend(
        surface
            .handles
            .iter()
            .filter(|h| h.ops.iter().any(|o| o.profiles.contains(&p.name)))
            .map(|h| area(&h.tag)),
    );
    list
}

#[cfg(test)]
mod tests {
    use super::{SwiftNaming, from_wire, has_int64};
    use crate::context::Naming as _;
    use crate::ir::Type;

    #[test]
    fn names_follow_swift() {
        assert_eq!(SwiftNaming.field_name("org_id"), "orgId");
        assert_eq!(SwiftNaming.field_name("default"), "`default`");
        assert_eq!(SwiftNaming.field_name("24h_count"), "_24hCount");
        assert_eq!(SwiftNaming.method_name("get_usage"), "getUsage");
        assert_eq!(SwiftNaming.type_name("acme-ci"), "AcmeCi");
    }

    #[test]
    fn int64_converts_through_the_wire_type() {
        let ty = Type::Array {
            item: Box::new(Type::Int64),
        };
        assert!(has_int64(&ty));
        assert_eq!(from_wire("x", &ty), "x.map { $0.value }");
        assert_eq!(from_wire("x", &Type::String), "x");
    }
}
