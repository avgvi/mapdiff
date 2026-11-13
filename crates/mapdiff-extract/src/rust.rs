//! Rust: maps tree-sitter-rust nodes onto scopes.

use mapdiff_core::{Access, Modifier, ScopeKind};
use tree_sitter::Node;

use crate::build::{Item, Raw, Segment, assemble, one_line, text};

pub fn is_comment(node: Node<'_>) -> bool {
    matches!(node.kind(), "line_comment" | "block_comment")
}

/// Comments and outer attributes attach to the item below them. Inner ones
/// (`//!`, `#![...]`) describe the enclosing file or module instead.
pub fn is_attachable(node: Node<'_>) -> bool {
    match node.kind() {
        "attribute_item" => true,
        "line_comment" | "block_comment" => node.child_by_field_name("inner").is_none(),
        _ => false,
    }
}

/// Where an item sits, which decides its kind and its default access.
#[derive(Clone)]
enum Parent {
    Module,
    Trait(Access),
    Impl { of_trait: bool, type_name: String },
    FnBody,
}

pub fn file_items<'t>(root: Node<'t>, source: &[u8]) -> Vec<Raw<'t>> {
    items_in(root, source, &Parent::Module)
}

fn items_in<'t>(container: Node<'t>, source: &[u8], parent: &Parent) -> Vec<Raw<'t>> {
    let mut cursor = container.walk();
    let items = container
        .named_children(&mut cursor)
        .map(|node| (node, classify(node, source, parent)))
        .collect();
    assemble(items, &|node| nested_items(node, source))
}

fn classify<'t>(node: Node<'t>, source: &[u8], parent: &Parent) -> Item<'t> {
    let field = |name: &str| node.child_by_field_name(name);
    let access = visibility(node, source);
    let test = has_test_attribute(node, source);

    let raw = match node.kind() {
        "line_comment" | "block_comment" | "attribute_item" | "inner_attribute_item" => {
            return Item::Trivia;
        }
        "empty_statement" => return Item::Owned,

        "mod_item" => {
            let Some(name) = field("name") else {
                return Item::Statement("item");
            };
            let body = field("body");
            Raw::new(ScopeKind::Module, "mod", node)
                .named(name, source)
                .segment(format!("mod {}", text(name, source)))
                .access(access)
                .body(body)
                .children(
                    body.map(|body| items_in(body, source, &Parent::Module))
                        .unwrap_or_default(),
                )
        }

        "struct_item" | "union_item" => {
            let Some(name) = field("name") else {
                return Item::Statement("item");
            };
            let lang_kind = if node.kind() == "union_item" {
                "union"
            } else {
                "struct"
            };
            // Tuple structs keep their fields in the header; only named
            // fields are scopes.
            let body = field("body").filter(|body| body.kind() == "field_declaration_list");
            Raw::new(ScopeKind::Class, lang_kind, node)
                .named(name, source)
                .access(access)
                .body(body)
                .children(body.map(|body| fields(body, source)).unwrap_or_default())
        }

        "enum_item" => {
            let Some(name) = field("name") else {
                return Item::Statement("item");
            };
            let body = field("body");
            Raw::new(ScopeKind::Enum, "enum", node)
                .named(name, source)
                .access(access)
                .body(body)
                .children(
                    body.map(|body| variants(body, source, access))
                        .unwrap_or_default(),
                )
        }

        "trait_item" => {
            let Some(name) = field("name") else {
                return Item::Statement("item");
            };
            let body = field("body");
            Raw::new(ScopeKind::Interface, "trait", node)
                .named(name, source)
                .access(access)
                .body(body)
                .children(
                    body.map(|body| items_in(body, source, &Parent::Trait(access)))
                        .unwrap_or_default(),
                )
        }

        "impl_item" => {
            let Some(target) = field("type") else {
                return Item::Statement("item");
            };
            let body = field("body");
            let inner = Parent::Impl {
                of_trait: field("trait").is_some(),
                type_name: base_type_name(target, source),
            };
            let mut raw = Raw::new(ScopeKind::Impl, "impl", node)
                .segment(impl_segment(node, body, source))
                // An impl is visible wherever its type and trait are.
                .access(Access::Public)
                .body(body)
                .children(
                    body.map(|body| items_in(body, source, &inner))
                        .unwrap_or_default(),
                );
            raw.name = Some(one_line(text(target, source)));
            raw.name_node = Some(target);
            raw
        }

        "function_item" | "function_signature_item" => {
            let Some(name) = field("name") else {
                return Item::Statement("item");
            };
            let body = field("body");
            let (kind, access) = match parent {
                Parent::Module | Parent::FnBody => (ScopeKind::Function, access),
                Parent::Trait(inherited) => (ScopeKind::Method, *inherited),
                Parent::Impl { of_trait: true, .. } => (ScopeKind::Method, Access::Public),
                Parent::Impl {
                    of_trait: false,
                    type_name,
                } => {
                    let kind = if is_constructor(node, type_name, source) {
                        ScopeKind::Constructor
                    } else {
                        ScopeKind::Method
                    };
                    (kind, access)
                }
            };
            let modifiers = function_modifiers(node, source);
            Raw::new(kind, "fn", node)
                .named(name, source)
                .segment(format!("{}()", text(name, source)))
                .access(access)
                .modifier_if(modifiers.contains("async"), Modifier::Async)
                .modifier_if(modifiers.contains("const"), Modifier::Const)
                .modifier_if(modifiers.contains("unsafe"), Modifier::Unsafe)
                .body(body)
                .children(
                    body.map(|body| nested_items(body, source))
                        .unwrap_or_default(),
                )
        }

        "const_item" | "static_item" => {
            let Some(name) = field("name") else {
                return Item::Statement("item");
            };
            let lang_kind = if node.kind() == "static_item" {
                "static"
            } else {
                "const"
            };
            Raw::new(ScopeKind::Constant, lang_kind, node)
                .named(name, source)
                .access(member_access(parent, access))
        }

        "type_item" | "associated_type" => {
            let Some(name) = field("name") else {
                return Item::Statement("item");
            };
            Raw::new(ScopeKind::TypeAlias, "type", node)
                .named(name, source)
                .access(member_access(parent, access))
        }

        "macro_definition" => {
            let Some(name) = field("name") else {
                return Item::Statement("item");
            };
            Raw::new(ScopeKind::Macro, "macro_rules!", node)
                .named(name, source)
                .segment(format!("{}!", text(name, source)))
        }

        "use_declaration" | "extern_crate_declaration" => {
            let lang_kind = if node.kind() == "use_declaration" {
                "use"
            } else {
                "extern crate"
            };
            let mut raw = Raw::new(ScopeKind::Import, lang_kind, node).access(access);
            raw.segment = Segment::Import;
            raw
        }

        // Each macro call at item level is a unit: often it defines a test
        // or a type, so calls are not merged into one block.
        "macro_invocation" => return Item::Standalone("macro call"),
        "expression_statement" => {
            let is_macro = node
                .named_child(0)
                .is_some_and(|child| child.kind() == "macro_invocation");
            return if is_macro {
                Item::Standalone("macro call")
            } else {
                Item::Statement("item")
            };
        }
        _ => return Item::Statement("item"),
    };

    Item::Scope(raw.modifier_if(test, Modifier::Test))
}

/// Trait items and items in a trait impl have no visibility keyword of their
/// own; they are as visible as the trait.
fn member_access(parent: &Parent, declared: Access) -> Access {
    match parent {
        Parent::Trait(inherited) => *inherited,
        Parent::Impl { of_trait: true, .. } => Access::Public,
        _ => declared,
    }
}

fn fields<'t>(body: Node<'t>, source: &[u8]) -> Vec<Raw<'t>> {
    let mut cursor = body.walk();
    body.named_children(&mut cursor)
        .filter(|node| node.kind() == "field_declaration")
        .filter_map(|node| {
            let name = node.child_by_field_name("name")?;
            Some(
                Raw::new(ScopeKind::Field, "field", node)
                    .named(name, source)
                    .access(visibility(node, source)),
            )
        })
        .collect()
}

fn variants<'t>(body: Node<'t>, source: &[u8], access: Access) -> Vec<Raw<'t>> {
    let mut cursor = body.walk();
    body.named_children(&mut cursor)
        .filter(|node| node.kind() == "enum_variant")
        .filter_map(|node| {
            let name = node.child_by_field_name("name")?;
            Some(
                Raw::new(ScopeKind::Variant, "variant", node)
                    .named(name, source)
                    .access(access),
            )
        })
        .collect()
}

/// Named items declared inside a function body or a statement. Closures are
/// not scopes, so the search passes through them.
fn nested_items<'t>(node: Node<'t>, source: &[u8]) -> Vec<Raw<'t>> {
    let mut found = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_item" | "struct_item" | "union_item" | "enum_item" | "trait_item"
            | "impl_item" | "mod_item" => {
                if let Item::Scope(raw) = classify(child, source, &Parent::FnBody) {
                    found.push(raw);
                }
            }
            // Token trees are unparsed macro input; nothing in them is an item.
            "token_tree" | "line_comment" | "block_comment" => {}
            _ => found.extend(nested_items(child, source)),
        }
    }
    found
}

fn visibility(node: Node<'_>, source: &[u8]) -> Access {
    let mut cursor = node.walk();
    let modifier = node
        .children(&mut cursor)
        .find(|child| child.kind() == "visibility_modifier");
    match modifier.map(|modifier| one_line(text(modifier, source))) {
        None => Access::Private,
        Some(text) if text == "pub" => Access::Public,
        Some(text) if text == "pub(self)" => Access::Private,
        Some(_) => Access::Restricted,
    }
}

fn function_modifiers(node: Node<'_>, source: &[u8]) -> String {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == "function_modifiers")
        .map(|modifiers| text(modifiers, source).to_owned())
        .unwrap_or_default()
}

/// `#[test]`, `#[tokio::test]` and `#[cfg(test)]` on the lines above an item.
fn has_test_attribute(node: Node<'_>, source: &[u8]) -> bool {
    let mut previous = node.prev_sibling();
    while let Some(attribute) = previous {
        match attribute.kind() {
            "attribute_item" => {
                let text = one_line(text(attribute, source)).replace(' ', "");
                if text == "#[test]" || text.ends_with("::test]") || text.starts_with("#[cfg(test)")
                {
                    return true;
                }
            }
            "line_comment" | "block_comment" => {}
            _ => return false,
        }
        previous = attribute.prev_sibling();
    }
    false
}

/// The impl header as an ID segment: generics kept, `where` clause dropped.
fn impl_segment(node: Node<'_>, body: Option<Node<'_>>, source: &[u8]) -> String {
    let mut cursor = node.walk();
    let end = node
        .children(&mut cursor)
        .find(|child| child.kind() == "where_clause")
        .or(body)
        .map(|stop| stop.start_byte())
        .unwrap_or(node.end_byte());
    let header = std::str::from_utf8(&source[node.start_byte()..end]).unwrap_or("impl");
    // `impl <T>` and `impl<T>` are the same header.
    one_line(header).replacen("impl <", "impl<", 1)
}

/// `Vec<T>` -> `Vec`, `fmt::Display` -> `Display`, `&'a Foo` -> `Foo`.
fn base_type_name(target: Node<'_>, source: &[u8]) -> String {
    let mut node = target;
    loop {
        let inner = match node.kind() {
            "generic_type" | "reference_type" => node.child_by_field_name("type"),
            "scoped_type_identifier" => node.child_by_field_name("name"),
            _ => None,
        };
        match inner {
            Some(inner) => node = inner,
            None => break,
        }
    }
    text(node, source).to_owned()
}

/// A guess from the signature: no `self` receiver, and the return type names
/// `Self` or the impl's own type.
fn is_constructor(node: Node<'_>, type_name: &str, source: &[u8]) -> bool {
    let Some(parameters) = node.child_by_field_name("parameters") else {
        return false;
    };
    let mut cursor = parameters.walk();
    if parameters
        .named_children(&mut cursor)
        .any(|parameter| parameter.kind() == "self_parameter")
    {
        return false;
    }
    let Some(return_type) = node.child_by_field_name("return_type") else {
        return false;
    };
    text(return_type, source)
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|word| word == "Self" || (!type_name.is_empty() && word == type_name))
}
