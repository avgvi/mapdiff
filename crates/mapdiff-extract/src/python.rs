//! Python: maps tree-sitter-python nodes onto scopes.

use std::collections::HashSet;

use mapdiff_core::{Access, Modifier, ScopeKind};
use tree_sitter::Node;

use crate::build::{Item, Raw, Segment, assemble, text};

pub fn is_comment(node: Node<'_>) -> bool {
    node.kind() == "comment"
}

pub fn is_attachable(node: Node<'_>) -> bool {
    node.kind() == "comment"
}

/// Where a statement sits, which decides what a definition in it becomes.
#[derive(Clone, Copy)]
enum Parent {
    Module,
    /// A class body. `variants` is set for enums, whose attributes are
    /// variants rather than fields.
    Class {
        variants: bool,
    },
    /// Inside a function body or a compound statement.
    Nested,
}

struct Context<'s> {
    source: &'s [u8],
    /// The names in `__all__`, when the module defines it.
    exported: Option<HashSet<String>>,
}

pub fn file_items<'t>(root: Node<'t>, source: &[u8]) -> Vec<Raw<'t>> {
    let context = Context {
        source,
        exported: dunder_all(root, source),
    };
    statements_in(root, &context, Parent::Module)
}

fn statements_in<'t>(container: Node<'t>, context: &Context<'_>, parent: Parent) -> Vec<Raw<'t>> {
    let mut cursor = container.walk();
    let items = container
        .named_children(&mut cursor)
        .map(|node| (node, classify(node, context, parent)))
        .collect();
    assemble(items, &|node| nested_definitions(node, context, parent))
}

fn classify<'t>(node: Node<'t>, context: &Context<'_>, parent: Parent) -> Item<'t> {
    let source = context.source;
    match node.kind() {
        "comment" => Item::Trivia,
        "pass_statement" => Item::Owned,

        "import_statement" | "import_from_statement" | "future_import_statement" => {
            let mut raw = Raw::new(ScopeKind::Import, "import", node);
            raw.segment = Segment::Import;
            Item::Scope(raw)
        }

        "function_definition" | "class_definition" | "decorated_definition" => {
            match definition(node, context, parent) {
                Some(raw) => Item::Scope(raw),
                None => Item::Statement("statements"),
            }
        }

        "type_alias_statement" => {
            let name = node
                .child_by_field_name("left")
                .and_then(|left| first_identifier(left));
            match name {
                Some(name) => Item::Scope(
                    Raw::new(ScopeKind::TypeAlias, "type", node)
                        .named(name, source)
                        .access(context.access(text(name, source), parent)),
                ),
                None => Item::Statement("statements"),
            }
        }

        "expression_statement" => {
            let Some(inner) = node.named_child(0) else {
                return Item::Owned;
            };
            match inner.kind() {
                // Docstrings and `...` placeholders belong to their parent.
                "string" | "ellipsis" => Item::Owned,
                "assignment" => match assignment(node, inner, context, parent) {
                    Some(raw) => Item::Scope(raw),
                    None => Item::Statement("statements"),
                },
                _ => Item::Statement("statements"),
            }
        }

        "if_statement" => Item::Standalone("if"),
        "try_statement" => Item::Standalone("try"),
        "with_statement" => Item::Standalone("with"),
        "for_statement" => Item::Standalone("for"),
        "while_statement" => Item::Standalone("while"),
        "match_statement" => Item::Standalone("match"),
        _ => Item::Statement("statements"),
    }
}

/// `name = value` or `name: type = value`, at module or class level.
fn assignment<'t>(
    statement: Node<'t>,
    assignment: Node<'t>,
    context: &Context<'_>,
    parent: Parent,
) -> Option<Raw<'t>> {
    let left = assignment.child_by_field_name("left")?;
    if left.kind() != "identifier" {
        return None;
    }
    let name = text(left, context.source);
    let (kind, lang_kind) = match parent {
        Parent::Module => (ScopeKind::Constant, "assignment"),
        Parent::Class { variants: true } if !name.starts_with('_') => {
            (ScopeKind::Variant, "member")
        }
        Parent::Class { .. } => (ScopeKind::Field, "attribute"),
        Parent::Nested => return None,
    };
    Some(
        Raw::new(kind, lang_kind, statement)
            .named(left, context.source)
            .access(context.access(name, parent)),
    )
}

/// A `def` or `class`, with or without decorators.
fn definition<'t>(outer: Node<'t>, context: &Context<'_>, parent: Parent) -> Option<Raw<'t>> {
    let source = context.source;
    let (inner, decorators) = if outer.kind() == "decorated_definition" {
        let mut cursor = outer.walk();
        let decorators: Vec<String> = outer
            .named_children(&mut cursor)
            .filter(|child| child.kind() == "decorator")
            .map(|decorator| decorator_name(decorator, source))
            .collect();
        (outer.child_by_field_name("definition")?, decorators)
    } else {
        (outer, Vec::new())
    };
    let name = inner.child_by_field_name("name")?;
    let name_text = text(name, source);
    let body = inner.child_by_field_name("body");
    let has = |wanted: &str| {
        decorators
            .iter()
            .any(|decorator| decorator == wanted || decorator.ends_with(&format!(".{wanted}")))
    };

    if inner.kind() == "class_definition" {
        let bases = inner
            .child_by_field_name("superclasses")
            .map(|bases| base_names(bases, source))
            .unwrap_or_default();
        let is_enum = bases.iter().any(|base| {
            matches!(
                base.as_str(),
                "Enum" | "IntEnum" | "StrEnum" | "Flag" | "IntFlag"
            )
        });
        let is_protocol = bases.iter().any(|base| base == "Protocol");
        let kind = if is_enum {
            ScopeKind::Enum
        } else if is_protocol {
            ScopeKind::Interface
        } else {
            ScopeKind::Class
        };
        let children = body
            .map(|body| statements_in(body, context, Parent::Class { variants: is_enum }))
            .unwrap_or_default();
        return Some(
            Raw::new(kind, "class", outer)
                .named(name, source)
                .access(context.access(name_text, parent))
                .modifier_if(name_text.starts_with("Test"), Modifier::Test)
                .body(body)
                .children(children),
        );
    }

    let is_async = {
        let mut cursor = inner.walk();
        inner
            .children(&mut cursor)
            .any(|child| child.kind() == "async")
    };
    let in_class = matches!(parent, Parent::Class { .. });
    let is_setter = decorators
        .iter()
        .any(|decorator| decorator.ends_with(".setter"));
    let is_getter = has("property") || has("cached_property");

    let (kind, lang_kind, segment) = if !in_class {
        (
            ScopeKind::Function,
            if is_async { "async def" } else { "def" },
            format!("{name_text}()"),
        )
    } else if name_text == "__init__" {
        (ScopeKind::Constructor, "def", format!("{name_text}()"))
    } else if is_getter {
        (ScopeKind::Method, "property", format!("get {name_text}()"))
    } else if is_setter {
        (ScopeKind::Method, "setter", format!("set {name_text}()"))
    } else if has("staticmethod") {
        (ScopeKind::Method, "staticmethod", format!("{name_text}()"))
    } else if has("classmethod") {
        (ScopeKind::Method, "classmethod", format!("{name_text}()"))
    } else {
        (
            ScopeKind::Method,
            if is_async { "async def" } else { "def" },
            format!("{name_text}()"),
        )
    };

    let mut children = body
        .map(|body| nested_definitions(body, context, Parent::Nested))
        .unwrap_or_default();
    if kind == ScopeKind::Constructor
        && let Some(body) = body
    {
        let receiver = first_parameter(inner, source);
        let mut seen = HashSet::new();
        instance_attributes(body, &receiver, source, &mut seen, &mut children);
    }

    Some(
        Raw::new(kind, lang_kind, outer)
            .named(name, source)
            .segment(segment)
            .access(context.access(name_text, parent))
            .modifier_if(is_async, Modifier::Async)
            .modifier_if(has("staticmethod"), Modifier::Static)
            .modifier_if(has("abstractmethod"), Modifier::Abstract)
            .modifier_if(
                name_text == "test" || name_text.starts_with("test_"),
                Modifier::Test,
            )
            .modifier_if(body.is_some_and(contains_yield), Modifier::Generator)
            .body(body)
            .children(children),
    )
}

impl Context<'_> {
    /// Leading underscore means private. At module level, a name left out of
    /// `__all__` is restricted.
    fn access(&self, name: &str, parent: Parent) -> Access {
        let dunder = name.starts_with("__") && name.ends_with("__") && name.len() > 4;
        if (name.starts_with('_') && !dunder) || matches!(parent, Parent::Nested) {
            return Access::Private;
        }
        match (&self.exported, parent) {
            (Some(exported), Parent::Module) if !dunder && !exported.contains(name) => {
                Access::Restricted
            }
            _ => Access::Public,
        }
    }
}

/// `def` and `class` statements nested anywhere inside `node`, not looking
/// inside the ones found. `parent` is where the names they define end up: a
/// definition under a module-level `if` is still a module-level name.
fn nested_definitions<'t>(node: Node<'t>, context: &Context<'_>, parent: Parent) -> Vec<Raw<'t>> {
    let mut found = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_definition" | "class_definition" | "decorated_definition" => {
                found.extend(definition(child, context, parent));
            }
            "comment" | "string" => {}
            _ => found.extend(nested_definitions(child, context, parent)),
        }
    }
    found
}

/// `self.name = ...` statements in a constructor body, first one per name.
fn instance_attributes<'t>(
    node: Node<'t>,
    receiver: &str,
    source: &[u8],
    seen: &mut HashSet<String>,
    out: &mut Vec<Raw<'t>>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_definition" | "class_definition" | "decorated_definition" | "lambda" => {}
            "expression_statement" => {
                let target = child
                    .named_child(0)
                    .filter(|inner| inner.kind() == "assignment")
                    .and_then(|assignment| assignment.child_by_field_name("left"))
                    .filter(|left| left.kind() == "attribute");
                let Some(target) = target else { continue };
                let (Some(object), Some(attribute)) = (
                    target.child_by_field_name("object"),
                    target.child_by_field_name("attribute"),
                ) else {
                    continue;
                };
                let name = text(attribute, source);
                if text(object, source) == receiver && seen.insert(name.to_owned()) {
                    let dunder = name.starts_with("__") && name.ends_with("__");
                    let access = if name.starts_with('_') && !dunder {
                        Access::Private
                    } else {
                        Access::Public
                    };
                    out.push(
                        Raw::new(ScopeKind::Field, "attribute", child)
                            .named(attribute, source)
                            .access(access),
                    );
                }
            }
            _ => instance_attributes(child, receiver, source, seen, out),
        }
    }
}

fn first_parameter(function: Node<'_>, source: &[u8]) -> String {
    function
        .child_by_field_name("parameters")
        .and_then(|parameters| parameters.named_child(0))
        .and_then(first_identifier)
        .map(|name| text(name, source).to_owned())
        .unwrap_or_else(|| String::from("self"))
}

fn first_identifier(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() == "identifier" {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor).find_map(first_identifier)
}

/// `@functools.wraps(func)` -> `functools.wraps`.
fn decorator_name(decorator: Node<'_>, source: &[u8]) -> String {
    let text = text(decorator, source).trim_start_matches('@').trim();
    text.split('(').next().unwrap_or(text).trim().to_owned()
}

/// The last name of each base class: `enum.Enum` -> `Enum`.
fn base_names(bases: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut cursor = bases.walk();
    bases
        .named_children(&mut cursor)
        .filter(|base| matches!(base.kind(), "identifier" | "attribute"))
        .map(|base| {
            let text = text(base, source);
            text.rsplit('.').next().unwrap_or(text).to_owned()
        })
        .collect()
}

fn contains_yield(node: Node<'_>) -> bool {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| match child.kind() {
            "yield" => true,
            "function_definition" | "class_definition" | "decorated_definition" | "lambda" => false,
            _ => contains_yield(child),
        })
}

/// The strings in a module-level `__all__ = [...]`.
fn dunder_all(root: Node<'_>, source: &[u8]) -> Option<HashSet<String>> {
    let mut cursor = root.walk();
    for statement in root.named_children(&mut cursor) {
        if statement.kind() != "expression_statement" {
            continue;
        }
        let Some(assignment) = statement
            .named_child(0)
            .filter(|node| node.kind() == "assignment")
        else {
            continue;
        };
        let (Some(left), Some(right)) = (
            assignment.child_by_field_name("left"),
            assignment.child_by_field_name("right"),
        ) else {
            continue;
        };
        if text(left, source) != "__all__" {
            continue;
        }
        let mut names = HashSet::new();
        collect_strings(right, source, &mut names);
        return Some(names);
    }
    None
}

fn collect_strings(node: Node<'_>, source: &[u8], out: &mut HashSet<String>) {
    if node.kind() == "string_content" {
        out.insert(text(node, source).to_owned());
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_strings(child, source, out);
    }
}
