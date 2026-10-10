//! Makes the models safe to grow. typify writes plain structs and enums; a struct
//! literal or an exhaustive `match` in a user's code would stop compiling the day a
//! spec sync adds a field or a value. So every struct with named fields and every enum
//! is `#[non_exhaustive]`, and a struct is built through code that survives a new
//! field:
//!
//! - a struct whose every field has a default derives `Default` and is built as
//!   `Model::default().with_field(value)`;
//! - a struct with a field that has no default (an enum without one, the runtime's
//!   `Code`) gets `Model::new(..)` taking those fields, the others start empty;
//! - every field has a `with_<field>` setter; an `Option` field's setter takes the
//!   inner value.

use std::collections::{BTreeMap, BTreeSet};

use quote::{format_ident, quote};
use syn::ext::IdentExt as _;
use syn::{Fields, GenericArgument, Item, PathArguments, Type};

/// Type names that always have a default, by the last segment of their path.
const DEFAULTED: &[&str] = &[
    "String", "bool", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64",
    "u128", "usize", "f32", "f64", "Option", "Vec", "BTreeMap", "HashMap", "Map", "Value", "Int64",
];

/// Applies the rules above to typify's output, in place.
pub(crate) fn apply(file: &mut syn::File) {
    let defaulted = defaulted_types(file);
    let mut impls = Vec::new();
    for item in &mut file.items {
        match item {
            Item::Struct(s) => {
                let Fields::Named(named) = &s.fields else {
                    continue;
                };
                let name = s.ident.clone();
                let mut required = Vec::new();
                let mut setters = Vec::new();
                let mut inits = Vec::new();
                for field in &named.named {
                    let Some(ident) = field.ident.clone() else {
                        continue;
                    };
                    let ty = &field.ty;
                    let doc = format!(" Sets `{}`.", ident.unraw());
                    let setter = format_ident!("with_{}", ident.unraw());
                    if let Some(inner) = option_inner(ty) {
                        setters.push(quote! {
                            #[doc = #doc]
                            #[must_use]
                            pub fn #setter(mut self, value: impl ::std::convert::Into<#inner>) -> Self {
                                self.#ident = ::std::option::Option::Some(value.into());
                                self
                            }
                        });
                    } else {
                        setters.push(quote! {
                            #[doc = #doc]
                            #[must_use]
                            pub fn #setter(mut self, value: impl ::std::convert::Into<#ty>) -> Self {
                                self.#ident = value.into();
                                self
                            }
                        });
                    }
                    if has_default(ty, &defaulted) {
                        inits.push(quote! { #ident: ::std::default::Default::default() });
                    } else {
                        required.push(quote! { #ident: impl ::std::convert::Into<#ty> });
                        inits.push(quote! { #ident: #ident.into() });
                    }
                }
                s.attrs.push(syn::parse_quote!(#[non_exhaustive]));
                let constructor = if defaulted.contains(&name.to_string()) {
                    if !derives_default(&s.attrs) {
                        s.attrs.push(syn::parse_quote!(#[derive(Default)]));
                    }
                    quote! {}
                } else {
                    let doc = format!(
                        " A `{name}` with the fields that have no default; set the others with the `with_` methods."
                    );
                    quote! {
                        #[doc = #doc]
                        #[must_use]
                        pub fn new(#(#required),*) -> Self {
                            Self { #(#inits),* }
                        }
                    }
                };
                impls.push(syn::parse_quote! {
                    impl #name {
                        #constructor
                        #(#setters)*
                    }
                });
            }
            Item::Enum(e) => e.attrs.push(syn::parse_quote!(#[non_exhaustive])),
            _ => {}
        }
    }
    file.items.extend(impls);
}

/// The generated types that have a default once this pass is done: enums and tuple
/// structs that already implement `Default`, and every struct with named fields whose
/// fields all have one (found as a fixed point, since structs nest).
fn defaulted_types(file: &syn::File) -> BTreeSet<String> {
    let mut known: BTreeSet<String> = BTreeSet::new();
    let mut named: BTreeMap<String, Vec<Type>> = BTreeMap::new();
    for item in &file.items {
        match item {
            Item::Struct(s) => match &s.fields {
                Fields::Named(f) => {
                    named.insert(
                        s.ident.to_string(),
                        f.named.iter().map(|f| f.ty.clone()).collect(),
                    );
                }
                _ if derives_default(&s.attrs) => {
                    known.insert(s.ident.to_string());
                }
                _ => {}
            },
            Item::Enum(e) if derives_default(&e.attrs) => {
                known.insert(e.ident.to_string());
            }
            Item::Impl(i)
                if i.trait_
                    .as_ref()
                    .is_some_and(|(p, _)| last_is(p, "Default")) =>
            {
                if let Type::Path(p) = &*i.self_ty
                    && let Some(seg) = p.path.segments.last()
                {
                    known.insert(seg.ident.to_string());
                }
            }
            _ => {}
        }
    }
    loop {
        let before = known.len();
        for (name, fields) in &named {
            if !known.contains(name) && fields.iter().all(|t| has_default(t, &known)) {
                known.insert(name.clone());
            }
        }
        if known.len() == before {
            return known;
        }
    }
}

fn has_default(ty: &Type, defaulted: &BTreeSet<String>) -> bool {
    let Type::Path(p) = ty else {
        return false;
    };
    let Some(seg) = p.path.segments.last() else {
        return false;
    };
    let name = seg.ident.to_string();
    if name == "Box" {
        return first_argument(&seg.arguments).is_some_and(|t| has_default(t, defaulted));
    }
    // A runtime replacement (`inorbithr::Code`) is named by a path of two or more
    // segments ending in a name the generated file does not define.
    if DEFAULTED.contains(&name.as_str()) && name != "Value" {
        return true;
    }
    if name == "Value" {
        // `serde_json::Value` has a default; the generated `Value` (protobuf) is
        // decided like any other model.
        return p.path.segments.len() > 1 || defaulted.contains(&name);
    }
    p.path.segments.len() == 1 && defaulted.contains(&name)
}

fn option_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(p) = ty else {
        return None;
    };
    let seg = p.path.segments.last()?;
    if seg.ident != "Option" {
        return None;
    }
    first_argument(&seg.arguments)
}

fn first_argument(args: &PathArguments) -> Option<&Type> {
    let PathArguments::AngleBracketed(a) = args else {
        return None;
    };
    a.args.iter().find_map(|g| match g {
        GenericArgument::Type(t) => Some(t),
        _ => None,
    })
}

fn last_is(path: &syn::Path, name: &str) -> bool {
    path.segments.last().is_some_and(|s| s.ident == name)
}

fn derives_default(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        if !a.path().is_ident("derive") {
            return false;
        }
        let mut found = false;
        let _ = a.parse_nested_meta(|m| {
            if last_is(&m.path, "Default") {
                found = true;
            }
            Ok(())
        });
        found
    })
}

#[cfg(test)]
mod tests {
    use super::apply;

    fn shaped(src: &str) -> String {
        let mut file: syn::File = syn::parse_str(src).unwrap();
        apply(&mut file);
        // Without whitespace and trailing commas, so the checks do not depend on where
        // prettyplease breaks a line.
        prettyplease::unparse(&file)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .replace(",)", ")")
    }

    fn has(out: &str, needle: &str) -> bool {
        out.contains(&needle.replace(char::is_whitespace, ""))
    }

    #[test]
    fn a_struct_of_defaults_derives_default_and_gets_setters() {
        let out = shaped(
            "#[derive(Clone, Debug, PartialEq)] pub struct Account { pub id: ::std::string::String, \
             pub when: ::std::option::Option<::std::string::String>, pub r#type: i32 }",
        );
        assert!(has(&out, "#[non_exhaustive]"), "{out}");
        assert!(has(&out, "#[derive(Default)]"), "{out}");
        assert!(!has(&out, "pub fn new("), "{out}");
        assert!(
            has(
                &out,
                "pub fn with_when(mut self, value: impl ::std::convert::Into<::std::string::String>)"
            ),
            "{out}"
        );
        assert!(
            has(
                &out,
                "self.when = ::std::option::Option::Some(value.into())"
            ),
            "{out}"
        );
        assert!(has(&out, "pub fn with_type("), "{out}");
    }

    #[test]
    fn a_field_without_a_default_goes_into_new() {
        let out = shaped(
            "#[derive(Clone, Copy)] pub enum Impact { A, B } \
             #[derive(Clone)] pub struct Change { pub impact: Impact, pub title: ::std::string::String, \
             pub code: inorbithr::Code } \
             #[derive(Clone, Default)] pub struct Holder { pub change: ::std::option::Option<Change> }",
        );
        assert!(
            has(
                &out,
                "pub fn new(impact: impl ::std::convert::Into<Impact>, code: impl ::std::convert::Into<inorbithr::Code>) -> Self"
            ),
            "{out}"
        );
        assert!(
            has(&out, "title: ::std::default::Default::default()"),
            "{out}"
        );
        assert_eq!(out.matches("#[non_exhaustive]").count(), 3, "{out}");
        assert!(!has(&out, "#[derive(Default)] pub struct Change"), "{out}");
    }

    #[test]
    fn a_struct_of_defaulted_structs_is_defaulted_too() {
        let out = shaped("pub struct Outer { pub inner: Inner } pub struct Inner { pub n: i32 }");
        assert_eq!(out.matches("#[derive(Default)]").count(), 2, "{out}");
        assert!(!has(&out, "pub fn new("), "{out}");
    }
}
