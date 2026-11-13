//! JavaScript and TypeScript: maps the tree-sitter-javascript and
//! tree-sitter-typescript nodes onto scopes. The two grammars share almost
//! all node names, so one extractor serves both, plus JSX and TSX.

use std::collections::HashSet;

use mapdiff_core::{Access, Modifier, ScopeKind};
use tree_sitter::Node;

use crate::build::{Item, Raw, Segment, assemble, text};

pub fn is_comment(node: Node<'_>) -> bool {
    matches!(node.kind(), "comment" | "html_comment")
}

pub fn is_attachable(node: Node<'_>) -> bool {
    matches!(node.kind(), "comment" | "decorator")
}

struct Context<'s> {
    source: &'s [u8],
    /// Functions that have `Name.prototype.x = ...` assignments in this file.
    /// They are constructor functions, which the model treats as classes.
    prototype_owners: HashSet<String>,
}

/// Names made public by a statement other than their own declaration:
/// `export { a }`, `export default a`, `module.exports = a`.
#[derive(Default)]
struct Exports {
    names: HashSet<String>,
    default: Option<String>,
}

pub fn file_items<'t>(root: Node<'t>, source: &[u8]) -> Vec<Raw<'t>> {
    let context = Context {
        source,
        prototype_owners: prototype_owners(root, source),
    };
    statements_in(root, &context)
}

/// The scopes among the direct children of a module or namespace body.
fn statements_in<'t>(container: Node<'t>, context: &Context<'_>) -> Vec<Raw<'t>> {
    let mut exports = Exports::default();
    let mut items = Vec::new();
    let mut cursor = container.walk();
    for node in container.named_children(&mut cursor) {
        items.push((node, classify(node, context, &mut exports)));
    }
    let mut scopes = assemble(items, &|node| nested_declarations(node, context));
    for scope in &mut scopes {
        let Some(name) = scope.name.as_deref() else {
            continue;
        };
        if exports.names.contains(name) {
            scope.access = Access::Public;
        }
        if exports.default.as_deref() == Some(name)
            && !scope.modifiers.contains(&Modifier::DefaultExport)
        {
            scope.modifiers.push(Modifier::DefaultExport);
        }
    }
    scopes
}

fn classify<'t>(node: Node<'t>, context: &Context<'_>, exports: &mut Exports) -> Item<'t> {
    match node.kind() {
        "comment" | "html_comment" | "decorator" => Item::Trivia,
        "hash_bang_line" | "empty_statement" => Item::Owned,
        "import_statement" => Item::Scope(import(node, "import", Access::Private)),
        "export_statement" => export(node, context, exports),
        "expression_statement" => expression(node, context, exports),
        "if_statement" => Item::Standalone("if"),
        "try_statement" => Item::Standalone("try"),
        "switch_statement" => Item::Standalone("switch"),
        "for_statement" | "for_in_statement" => Item::Standalone("for"),
        "while_statement" | "do_statement" => Item::Standalone("while"),
        _ => match declaration(node, node, context, Access::Private) {
            Some(raw) => Item::Scope(raw),
            None => Item::Statement("statements"),
        },
    }
}

fn import<'t>(node: Node<'t>, lang_kind: &'static str, access: Access) -> Raw<'t> {
    let mut raw = Raw::new(ScopeKind::Import, lang_kind, node).access(access);
    raw.segment = Segment::Import;
    raw
}

/// A declaration statement. `outer` is the whole statement including any
/// `export` or `declare` in front; `node` is the declaration itself.
fn declaration<'t>(
    outer: Node<'t>,
    node: Node<'t>,
    context: &Context<'_>,
    access: Access,
) -> Option<Raw<'t>> {
    let source = context.source;
    let field = |name: &str| node.child_by_field_name(name);

    let raw = match node.kind() {
        "function_declaration" | "generator_function_declaration" | "function_signature" => {
            let name = field("name")?;
            let name_text = text(name, source);
            let is_class = context.prototype_owners.contains(name_text);
            let lang_kind = if node.kind() == "function_signature" {
                "overload"
            } else {
                "function"
            };
            let mut raw = function_like(outer, node, context, lang_kind)
                .named(name, source)
                .access(access);
            if is_class {
                raw.kind = ScopeKind::Class;
            } else {
                raw.segment = Segment::Name(format!("{name_text}()"));
            }
            raw
        }

        "class_declaration" | "abstract_class_declaration" | "class" => {
            let mut raw = class(outer, node, context).access(access);
            if let Some(name) = field("name") {
                raw = raw.named(name, source);
            }
            raw
        }

        "interface_declaration" => {
            let name = field("name")?;
            let body = field("body");
            Raw::new(ScopeKind::Interface, "interface", outer)
                .named(name, source)
                .access(access)
                .body(body)
                .children(
                    body.map(|body| interface_members(body, source, access))
                        .unwrap_or_default(),
                )
        }

        "type_alias_declaration" => Raw::new(ScopeKind::TypeAlias, "type", outer)
            .named(field("name")?, source)
            .access(access),

        "enum_declaration" => {
            let name = field("name")?;
            let body = field("body");
            Raw::new(ScopeKind::Enum, "enum", outer)
                .named(name, source)
                .access(access)
                .body(body)
                .children(
                    body.map(|body| enum_members(body, source, access))
                        .unwrap_or_default(),
                )
        }

        "internal_module" | "module" => {
            let name = field("name")?;
            let body = field("body");
            Raw::new(ScopeKind::Module, "namespace", outer)
                .named(name, source)
                .segment(format!("namespace {}", text(name, source)))
                .access(access)
                .body(body)
                .children(
                    body.map(|body| statements_in(body, context))
                        .unwrap_or_default(),
                )
        }

        "ambient_declaration" => {
            let mut cursor = node.walk();
            let inner = node
                .named_children(&mut cursor)
                .find(|child| !is_comment(*child))?;
            let raw = if inner.kind() == "statement_block" {
                // `declare global { ... }`
                let mut raw = Raw::new(ScopeKind::Module, "declare global", outer)
                    .segment("namespace global")
                    .access(access)
                    .body(Some(inner))
                    .children(statements_in(inner, context));
                raw.name = Some(String::from("global"));
                raw
            } else {
                declaration(outer, inner, context, access)?
            };
            raw.modifier(Modifier::Declare)
        }

        "lexical_declaration" | "variable_declaration" => variable(outer, node, context, access)?,

        _ => return None,
    };
    Some(raw)
}

/// `const name = value`. What it becomes depends on the value.
fn variable<'t>(
    outer: Node<'t>,
    node: Node<'t>,
    context: &Context<'_>,
    access: Access,
) -> Option<Raw<'t>> {
    let source = context.source;
    let keyword = match node.child(0).map(|child| child.kind()) {
        Some("let") => "let",
        Some("var") => "var",
        _ => "const",
    };
    let mut cursor = node.walk();
    let declarator = node
        .named_children(&mut cursor)
        .find(|child| child.kind() == "variable_declarator")?;
    let name = declarator.child_by_field_name("name")?;
    let value = declarator.child_by_field_name("value");

    if value.is_some_and(|value| is_require(value, source)) {
        let mut raw = import(outer, "require", access);
        if name.kind() == "identifier" {
            raw.name = Some(text(name, source).to_owned());
            raw.name_node = Some(name);
        }
        return Some(raw);
    }
    if name.kind() != "identifier" {
        return None;
    }
    let name_text = text(name, source);

    let raw = match value {
        Some(value) if is_function(value) => function_like(outer, value, context, keyword)
            .named(name, source)
            .segment(format!("{name_text}()")),
        Some(value) if value.kind() == "class" => class(outer, value, context).named(name, source),
        Some(value) if value.kind() == "object" => Raw::new(ScopeKind::Constant, keyword, outer)
            .named(name, source)
            .body(Some(value))
            .children(object_members(value, context)),
        _ => Raw::new(ScopeKind::Constant, keyword, outer).named(name, source),
    };
    Some(raw.access(access))
}

fn is_function(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "arrow_function" | "function_expression" | "generator_function" | "function"
    )
}

/// A function scope from a function-like node. The caller sets the name.
fn function_like<'t>(
    outer: Node<'t>,
    function: Node<'t>,
    context: &Context<'_>,
    lang_kind: &'static str,
) -> Raw<'t> {
    let body = function.child_by_field_name("body");
    Raw::new(ScopeKind::Function, lang_kind, outer)
        .modifier_if(has_token(function, "async"), Modifier::Async)
        .modifier_if(
            function.kind().contains("generator") || has_token(function, "*"),
            Modifier::Generator,
        )
        .body(body)
        .children(
            body.map(|body| nested_declarations(body, context))
                .unwrap_or_default(),
        )
}

/// A class scope without a name. The caller sets the name or segment.
fn class<'t>(outer: Node<'t>, node: Node<'t>, context: &Context<'_>) -> Raw<'t> {
    let body = node.child_by_field_name("body");
    Raw::new(ScopeKind::Class, "class", outer)
        .segment("{default}")
        .modifier_if(
            node.kind() == "abstract_class_declaration",
            Modifier::Abstract,
        )
        .body(body)
        .children(
            body.map(|body| class_members(body, context))
                .unwrap_or_default(),
        )
}

fn class_members<'t>(body: Node<'t>, context: &Context<'_>) -> Vec<Raw<'t>> {
    let source = context.source;
    let mut members = Vec::new();
    let mut cursor = body.walk();
    for node in body.named_children(&mut cursor) {
        match node.kind() {
            "method_definition" | "abstract_method_signature" | "method_signature" => {
                members.extend(method(node, context));
            }
            "public_field_definition" | "field_definition" => {
                let Some(name) = node
                    .child_by_field_name("name")
                    .or_else(|| node.child_by_field_name("property"))
                else {
                    continue;
                };
                let value = node
                    .child_by_field_name("value")
                    .filter(|v| is_function(*v));
                // A field holding a function is a method written as a property.
                let raw = match value {
                    Some(value) => {
                        let mut raw = function_like(node, value, context, "property")
                            .named(name, source)
                            .segment(format!("{}()", text(name, source)));
                        raw.kind = ScopeKind::Method;
                        raw
                    }
                    None => Raw::new(ScopeKind::Field, "property", node).named(name, source),
                };
                members.push(member_modifiers(raw, node, name, source));
            }
            _ => {}
        }
    }
    members
}

/// A method, constructor, getter or setter in a class or object literal.
fn method<'t>(node: Node<'t>, context: &Context<'_>) -> Option<Raw<'t>> {
    let source = context.source;
    let name = node.child_by_field_name("name")?;
    let name_text = text(name, source);
    let body = node.child_by_field_name("body");

    let (kind, lang_kind, segment) = if name_text == "constructor" {
        (
            ScopeKind::Constructor,
            "constructor",
            String::from("constructor()"),
        )
    } else if has_token(node, "get") {
        (ScopeKind::Method, "getter", format!("get {name_text}()"))
    } else if has_token(node, "set") {
        (ScopeKind::Method, "setter", format!("set {name_text}()"))
    } else {
        (ScopeKind::Method, "method", format!("{name_text}()"))
    };

    let mut children = body
        .map(|body| nested_declarations(body, context))
        .unwrap_or_default();
    if kind == ScopeKind::Constructor
        && let Some(parameters) = node.child_by_field_name("parameters")
    {
        children.extend(parameter_properties(parameters, source));
    }

    let raw = Raw::new(kind, lang_kind, node)
        .named(name, source)
        .segment(segment)
        .modifier_if(has_token(node, "async"), Modifier::Async)
        .modifier_if(has_token(node, "*"), Modifier::Generator)
        .modifier_if(
            node.kind() == "abstract_method_signature",
            Modifier::Abstract,
        )
        .body(body)
        .children(children);
    Some(member_modifiers(raw, node, name, source))
}

/// Access and the `static`, `readonly`, `override`, `abstract`, `declare`
/// flags of a class member.
fn member_modifiers<'t>(raw: Raw<'t>, node: Node<'_>, name: Node<'_>, source: &[u8]) -> Raw<'t> {
    let mut cursor = node.walk();
    let declared = node
        .children(&mut cursor)
        .find(|child| child.kind() == "accessibility_modifier")
        .map(|modifier| text(modifier, source).to_owned());
    let access = match declared.as_deref() {
        Some("private") => Access::Private,
        Some("protected") => Access::Restricted,
        _ if name.kind() == "private_property_identifier" => Access::Private,
        _ => Access::Public,
    };
    raw.access(access)
        .modifier_if(has_token(node, "static"), Modifier::Static)
        .modifier_if(has_token(node, "readonly"), Modifier::Readonly)
        .modifier_if(has_token(node, "abstract"), Modifier::Abstract)
        .modifier_if(has_token(node, "declare"), Modifier::Declare)
        .modifier_if(has_token(node, "override_modifier"), Modifier::Override)
}

/// `constructor(public radius: number)`: a parameter that is also a field.
fn parameter_properties<'t>(parameters: Node<'t>, source: &[u8]) -> Vec<Raw<'t>> {
    let mut cursor = parameters.walk();
    parameters
        .named_children(&mut cursor)
        .filter(|parameter| {
            has_token(*parameter, "accessibility_modifier") || has_token(*parameter, "readonly")
        })
        .filter_map(|parameter| {
            let name = parameter
                .child_by_field_name("pattern")
                .filter(|pattern| pattern.kind() == "identifier")?;
            let raw =
                Raw::new(ScopeKind::Field, "parameter property", parameter).named(name, source);
            Some(member_modifiers(raw, parameter, name, source))
        })
        .collect()
}

fn interface_members<'t>(body: Node<'t>, source: &[u8], access: Access) -> Vec<Raw<'t>> {
    let mut cursor = body.walk();
    body.named_children(&mut cursor)
        .filter_map(|node| {
            let name = node.child_by_field_name("name")?;
            let raw = match node.kind() {
                "property_signature" => {
                    Raw::new(ScopeKind::Field, "property", node).named(name, source)
                }
                "method_signature" => Raw::new(ScopeKind::Method, "method", node)
                    .named(name, source)
                    .segment(format!("{}()", text(name, source))),
                _ => return None,
            };
            Some(
                raw.access(access)
                    .modifier_if(has_token(node, "readonly"), Modifier::Readonly),
            )
        })
        .collect()
}

fn enum_members<'t>(body: Node<'t>, source: &[u8], access: Access) -> Vec<Raw<'t>> {
    let mut cursor = body.walk();
    body.named_children(&mut cursor)
        .filter_map(|node| {
            let name = match node.kind() {
                "enum_assignment" => node.child_by_field_name("name")?,
                "property_identifier" => node,
                _ => return None,
            };
            Some(
                Raw::new(ScopeKind::Variant, "member", node)
                    .named(name, source)
                    .access(access),
            )
        })
        .collect()
}

/// The functions in an object literal bound to a name.
fn object_members<'t>(object: Node<'t>, context: &Context<'_>) -> Vec<Raw<'t>> {
    let source = context.source;
    let mut cursor = object.walk();
    object
        .named_children(&mut cursor)
        .filter_map(|node| match node.kind() {
            "method_definition" => method(node, context),
            "pair" => {
                let key = node
                    .child_by_field_name("key")
                    .filter(|key| key.kind() == "property_identifier")?;
                let value = node
                    .child_by_field_name("value")
                    .filter(|v| is_function(*v))?;
                let mut raw = function_like(node, value, context, "method")
                    .named(key, source)
                    .segment(format!("{}()", text(key, source)))
                    .access(Access::Public);
                raw.kind = ScopeKind::Method;
                Some(raw)
            }
            _ => None,
        })
        .collect()
}

fn export<'t>(node: Node<'t>, context: &Context<'_>, exports: &mut Exports) -> Item<'t> {
    let source = context.source;
    let is_default = has_token(node, "default");

    if let Some(inner) = node.child_by_field_name("declaration") {
        return match declaration(node, inner, context, Access::Public) {
            Some(raw) => Item::Scope(raw.modifier_if(is_default, Modifier::DefaultExport)),
            None => Item::Statement("statements"),
        };
    }

    // `export ... from "module"` re-exports without defining anything here.
    if node.child_by_field_name("source").is_some() {
        return Item::Scope(import(node, "export from", Access::Public));
    }

    if let Some(value) = node.child_by_field_name("value") {
        let raw = if is_function(value) {
            let mut raw = function_like(node, value, context, "function").segment("{default}");
            if let Some(name) = value.child_by_field_name("name") {
                raw = raw
                    .named(name, source)
                    .segment(format!("{}()", text(name, source)));
            }
            raw
        } else if value.kind() == "class" {
            let mut raw = class(node, value, context);
            if let Some(name) = value.child_by_field_name("name") {
                raw = raw.named(name, source);
            }
            raw
        } else if value.kind() == "identifier" {
            // `export default Name;` marks an existing scope.
            let name = text(value, source).to_owned();
            exports.names.insert(name.clone());
            exports.default = Some(name);
            return Item::Owned;
        } else if value.kind() == "object" {
            Raw::new(ScopeKind::Constant, "export default", node)
                .segment("{default}")
                .body(Some(value))
                .children(object_members(value, context))
        } else {
            Raw::new(ScopeKind::Constant, "export default", node).segment("{default}")
        };
        return Item::Scope(raw.access(Access::Public).modifier(Modifier::DefaultExport));
    }

    // `export { a, b as c };` marks existing scopes.
    let mut cursor = node.walk();
    for clause in node.named_children(&mut cursor) {
        if clause.kind() != "export_clause" {
            continue;
        }
        let mut inner = clause.walk();
        for specifier in clause.named_children(&mut inner) {
            if let Some(name) = specifier.child_by_field_name("name") {
                exports.names.insert(text(name, source).to_owned());
            }
        }
    }
    Item::Owned
}

fn expression<'t>(node: Node<'t>, context: &Context<'_>, exports: &mut Exports) -> Item<'t> {
    let Some(inner) = node.named_child(0) else {
        return Item::Owned;
    };
    match inner.kind() {
        // `'use strict'` and other directives belong to the file.
        "string" => Item::Owned,
        "internal_module" => match declaration(node, inner, context, Access::Private) {
            Some(raw) => Item::Scope(raw),
            None => Item::Statement("statements"),
        },
        "assignment_expression" => common_js(node, inner, context, exports),
        "call_expression" => {
            let callee = inner
                .child_by_field_name("function")
                .filter(|callee| callee.kind() == "parenthesized_expression")
                .and_then(|callee| callee.named_child(0));
            if callee.is_some_and(is_function) {
                Item::Standalone("iife")
            } else {
                Item::Statement("statements")
            }
        }
        _ => Item::Statement("statements"),
    }
}

/// `module.exports = ...`, `exports.name = ...` and `Name.prototype.x = ...`.
fn common_js<'t>(
    statement: Node<'t>,
    assignment: Node<'t>,
    context: &Context<'_>,
    exports: &mut Exports,
) -> Item<'t> {
    let source = context.source;
    let (Some(left), Some(right)) = (
        assignment.child_by_field_name("left"),
        assignment.child_by_field_name("right"),
    ) else {
        return Item::Statement("statements");
    };
    let Some(chain) = member_chain(left, source) else {
        return Item::Statement("statements");
    };
    let chain: Vec<&str> = chain.iter().map(String::as_str).collect();
    let property = left.child_by_field_name("property");

    match chain.as_slice() {
        // The whole export.
        ["module", "exports"] => {
            let raw = if is_function(right) {
                let mut raw =
                    function_like(statement, right, context, "module.exports").segment("{exports}");
                if let Some(name) = right.child_by_field_name("name") {
                    raw = raw
                        .named(name, source)
                        .segment(format!("{}()", text(name, source)));
                }
                raw
            } else if right.kind() == "class" {
                let mut raw = class(statement, right, context).segment("{exports}");
                if let Some(name) = right.child_by_field_name("name") {
                    raw = raw.named(name, source);
                }
                raw
            } else if right.kind() == "object" {
                Raw::new(ScopeKind::Constant, "module.exports", statement)
                    .segment("{exports}")
                    .body(Some(right))
                    .children(object_members(right, context))
            } else {
                if right.kind() == "identifier" {
                    // `module.exports = name;` makes `name` what a default
                    // import or a bare `require` of this file gives.
                    let name = text(right, source).to_owned();
                    exports.names.insert(name.clone());
                    exports.default = Some(name);
                }
                return Item::Statement("exports");
            };
            Item::Scope(raw.access(Access::Public))
        }

        // One named export.
        ["module", "exports", _] | ["exports", _] => {
            let Some(property) = property else {
                return Item::Statement("exports");
            };
            let raw = if is_function(right) {
                function_like(statement, right, context, "exports")
                    .named(property, source)
                    .segment(format!("{}()", text(property, source)))
            } else if right.kind() == "class" {
                class(statement, right, context).named(property, source)
            } else {
                if right.kind() == "identifier" {
                    exports.names.insert(text(right, source).to_owned());
                }
                return Item::Statement("exports");
            };
            Item::Scope(raw.access(Access::Public))
        }

        // A method added to a constructor function.
        [owner, "prototype", name] if is_function(right) => {
            let Some(property) = property else {
                return Item::Statement("statements");
            };
            let mut raw = function_like(statement, right, context, "prototype method")
                .named(property, source)
                .segment(format!("{owner}.prototype.{name}()"))
                .access(Access::Public);
            raw.kind = ScopeKind::Method;
            Item::Scope(raw)
        }

        _ => Item::Statement("statements"),
    }
}

/// `a.b.c` as `["a", "b", "c"]`. `None` for anything else.
fn member_chain(node: Node<'_>, source: &[u8]) -> Option<Vec<String>> {
    match node.kind() {
        "identifier" => Some(vec![text(node, source).to_owned()]),
        "member_expression" => {
            let mut chain = member_chain(node.child_by_field_name("object")?, source)?;
            let property = node.child_by_field_name("property")?;
            if property.kind() != "property_identifier" {
                return None;
            }
            chain.push(text(property, source).to_owned());
            Some(chain)
        }
        _ => None,
    }
}

/// True for `require(...)`, `require(...).x` and `require(...)(...)`.
fn is_require(value: Node<'_>, source: &[u8]) -> bool {
    let mut node = value;
    loop {
        let next = match node.kind() {
            "call_expression" => {
                let Some(callee) = node.child_by_field_name("function") else {
                    return false;
                };
                if callee.kind() == "identifier" {
                    return text(callee, source) == "require";
                }
                Some(callee)
            }
            "member_expression" => node.child_by_field_name("object"),
            _ => None,
        };
        match next {
            Some(next) => node = next,
            None => return false,
        }
    }
}

/// Named function and class declarations anywhere inside `node`, not looking
/// inside the ones found. Callbacks and local arrow functions are not scopes.
fn nested_declarations<'t>(node: Node<'t>, context: &Context<'_>) -> Vec<Raw<'t>> {
    let mut found = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_declaration"
            | "generator_function_declaration"
            | "class_declaration"
            | "abstract_class_declaration" => {
                found.extend(declaration(child, child, context, Access::Private));
            }
            "comment" | "string" | "template_string" | "regex" => {}
            _ => found.extend(nested_declarations(child, context)),
        }
    }
    found
}

fn prototype_owners(root: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut owners = HashSet::new();
    let mut cursor = root.walk();
    for statement in root.named_children(&mut cursor) {
        if statement.kind() != "expression_statement" {
            continue;
        }
        let chain = statement
            .named_child(0)
            .filter(|inner| inner.kind() == "assignment_expression")
            .and_then(|assignment| assignment.child_by_field_name("left"))
            .and_then(|left| member_chain(left, source));
        if let Some(chain) = chain
            && chain.len() == 3
            && chain[1] == "prototype"
        {
            owners.insert(chain[0].clone());
        }
    }
    owners
}

/// True when `node` has a direct child of this kind. Keywords such as
/// `static` and `async` are unnamed children whose kind is their text.
fn has_token(node: Node<'_>, kind: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor).any(|child| child.kind() == kind)
}

/// Test files by naming convention: `*.test.*`, `*.spec.*`, `__tests__/`.
pub fn is_test_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.contains(".test.") || name.contains(".spec.") || path.contains("__tests__/")
}
