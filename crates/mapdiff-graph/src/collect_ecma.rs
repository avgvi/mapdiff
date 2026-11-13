//! JavaScript and TypeScript: reads uses, imports and base classes off a
//! syntax tree. One collector serves both grammars, plus JSX and TSX.

use std::collections::HashSet;

use mapdiff_core::RefKind;
use tree_sitter::Node;

use crate::facts::{Bases, Binding, Facts, Locals, SELF, SUPER, Seg, Target, string_value, text};

pub fn collect(root: Node<'_>, source: &[u8]) -> Facts {
    let mut collector = Collector {
        source,
        facts: Facts::default(),
        locals: Locals::default(),
    };
    collector.visit(root);
    collector.facts
}

struct Collector<'s> {
    source: &'s [u8],
    facts: Facts,
    locals: Locals,
}

fn is_function(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration"
            | "generator_function_declaration"
            | "function_expression"
            | "generator_function"
            | "arrow_function"
            | "method_definition"
            | "function_signature"
            | "method_signature"
            | "abstract_method_signature"
    )
}

fn is_class(kind: &str) -> bool {
    matches!(
        kind,
        "class_declaration" | "abstract_class_declaration" | "class"
    )
}

impl Collector<'_> {
    fn visit_children(&mut self, node: Node<'_>) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.visit(child);
        }
    }

    fn visit_field(&mut self, node: Node<'_>, field: &str) {
        if let Some(child) = node.child_by_field_name(field) {
            self.visit(child);
        }
    }

    fn visit(&mut self, node: Node<'_>) {
        let kind = node.kind();
        match kind {
            "comment" | "html_comment" | "string" | "regex" | "number" | "hash_bang_line" => {}
            // Names that label something instead of referring to something.
            "property_identifier"
            | "private_property_identifier"
            | "shorthand_property_identifier_pattern"
            | "statement_identifier"
            | "jsx_closing_element"
            | "this"
            | "super"
            | "break_statement"
            | "continue_statement" => {}

            "template_string" => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "template_substitution" {
                        self.visit_children(child);
                    }
                }
            }

            "import_statement" => self.import(node),
            "export_statement" if node.child_by_field_name("source").is_some() => {
                self.reexport(node)
            }

            "lexical_declaration" | "variable_declaration" => {
                let mut cursor = node.walk();
                for declarator in node.named_children(&mut cursor) {
                    if declarator.kind() == "variable_declarator" {
                        self.declarator(declarator);
                    }
                }
            }

            _ if is_function(kind) => self.function(node),
            _ if is_class(kind) => self.class(node),

            "interface_declaration" => {
                self.locals.push(type_parameter_names(node, self.source));
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    match child.kind() {
                        "extends_type_clause" => {
                            self.heritage(child, child.start_byte(), RefKind::Inherit)
                        }
                        "interface_body" | "object_type" => self.visit_children(child),
                        _ => {}
                    }
                }
                self.locals.pop();
            }

            "type_alias_declaration" => {
                self.locals.push(type_parameter_names(node, self.source));
                self.visit_field(node, "value");
                self.locals.pop();
            }

            "enum_declaration" => {
                if let Some(body) = node.child_by_field_name("body") {
                    let mut cursor = body.walk();
                    for member in body.named_children(&mut cursor) {
                        self.visit_field(member, "value");
                    }
                }
            }

            "internal_module" | "module" => self.visit_field(node, "body"),

            "call_expression" => {
                if let Some(function) = node.child_by_field_name("function") {
                    if let Some(specifier) = require_specifier(node, self.source) {
                        // A bare `require("./x")` binds nothing but depends on it.
                        self.bind(None, specifier, None, false, false, node, function);
                    } else {
                        match self.path(function) {
                            Some(path) => self.add(path, RefKind::Call),
                            None => self.visit(function),
                        }
                    }
                }
                self.visit_field(node, "type_arguments");
                self.visit_field(node, "arguments");
            }

            "new_expression" => {
                if let Some(constructor) = node.child_by_field_name("constructor") {
                    match self.path(constructor) {
                        Some(path) => self.add(path, RefKind::Call),
                        None => self.visit(constructor),
                    }
                }
                self.visit_field(node, "type_arguments");
                self.visit_field(node, "arguments");
            }

            "member_expression" => match self.path(node) {
                Some(path) => self.add(path, RefKind::Access),
                // The chain starts at a call or another expression: only
                // that part can hold uses.
                None => self.visit_field(node, "object"),
            },

            "identifier" | "shorthand_property_identifier" => {
                self.add(vec![Seg::of(node, self.source)], RefKind::Access);
            }
            "type_identifier" => self.add(vec![Seg::of(node, self.source)], RefKind::TypeUse),
            "nested_type_identifier" | "nested_identifier" => {
                if let Some(path) = self.path(node) {
                    self.add(path, RefKind::TypeUse);
                }
            }

            // An object key is a label; a computed key is an expression.
            "pair" => {
                if let Some(key) = node.child_by_field_name("key")
                    && key.kind() == "computed_property_name"
                {
                    self.visit(key);
                }
                self.visit_field(node, "value");
            }

            "jsx_opening_element" | "jsx_self_closing_element" => {
                if let Some(name) = node.child_by_field_name("name")
                    && let Some(path) = self.path(name)
                {
                    // Lower-case tags are HTML elements, not components.
                    let is_component = path.len() > 1
                        || path[0].name.chars().next().is_some_and(char::is_uppercase);
                    if is_component {
                        self.add(path, RefKind::Call);
                    }
                }
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "jsx_attribute" || child.kind() == "jsx_expression" {
                        self.visit(child);
                    }
                }
            }

            "catch_clause" => self.visit_field(node, "body"),

            // `for (const x of xs)` binds `x`; `for (x of xs)` uses it.
            "for_in_statement" => {
                if node.child_by_field_name("kind").is_none() {
                    self.visit_field(node, "left");
                }
                self.visit_field(node, "right");
                self.visit_field(node, "body");
            }

            _ => self.visit_children(node),
        }
    }

    /// `const name = value`. A `require` call becomes an import binding.
    fn declarator(&mut self, declarator: Node<'_>) {
        let name = declarator.child_by_field_name("name");
        let value = declarator.child_by_field_name("value");
        if let (Some(name), Some(value)) = (name, value)
            && let Some(specifier) = require_specifier(value, self.source)
        {
            match name.kind() {
                "identifier" => {
                    let local = text(name, self.source).to_owned();
                    self.bind(Some(local), specifier, None, false, false, declarator, name);
                }
                "object_pattern" => {
                    let mut cursor = name.walk();
                    for property in name.named_children(&mut cursor) {
                        match property.kind() {
                            // `const { a } = require(...)`
                            "shorthand_property_identifier_pattern" => {
                                let imported = text(property, self.source).to_owned();
                                self.bind(
                                    Some(imported.clone()),
                                    specifier.clone(),
                                    Some(imported),
                                    false,
                                    false,
                                    declarator,
                                    property,
                                );
                            }
                            // `const { a: b } = require(...)`
                            "pair_pattern" => {
                                let key = property.child_by_field_name("key");
                                let local = property
                                    .child_by_field_name("value")
                                    .filter(|value| value.kind() == "identifier");
                                if let (Some(key), Some(local)) = (key, local) {
                                    self.bind(
                                        Some(text(local, self.source).to_owned()),
                                        specifier.clone(),
                                        Some(text(key, self.source).to_owned()),
                                        false,
                                        false,
                                        declarator,
                                        local,
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            return;
        }
        self.visit_field(declarator, "type");
        if let Some(value) = value {
            self.visit(value);
        }
    }

    fn function(&mut self, node: Node<'_>) {
        let mut names = type_parameter_names(node, self.source);
        // A named function expression can refer to itself by that name.
        if matches!(node.kind(), "function_expression" | "generator_function")
            && let Some(name) = node.child_by_field_name("name")
        {
            names.insert(text(name, self.source).to_owned());
        }
        let parameters = node
            .child_by_field_name("parameters")
            .or_else(|| node.child_by_field_name("parameter"));
        if let Some(parameters) = parameters {
            pattern_names(parameters, self.source, &mut names);
        }
        let body = node.child_by_field_name("body");
        if let Some(body) = body {
            declared_names(body, self.source, &mut names);
        }

        self.locals.push(names);
        // Annotations and default values of parameters are uses.
        if let Some(parameters) = parameters {
            self.parameter_uses(parameters);
        }
        self.visit_field(node, "return_type");
        if let Some(body) = body {
            self.visit(body);
        }
        // A computed method name, `[key]() {}`, is an expression.
        if let Some(name) = node.child_by_field_name("name")
            && name.kind() == "computed_property_name"
        {
            self.visit(name);
        }
        self.locals.pop();
    }

    fn parameter_uses(&mut self, node: Node<'_>) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "type_annotation" | "decorator" => self.visit(child),
                "identifier" | "shorthand_property_identifier_pattern" | "this" => {}
                "assignment_pattern" | "object_assignment_pattern" => {
                    if let Some(left) = child.child_by_field_name("left") {
                        self.parameter_uses(left);
                    }
                    self.visit_field(child, "right");
                }
                "required_parameter" | "optional_parameter" => {
                    self.parameter_uses(child);
                    self.visit_field(child, "value");
                }
                _ => self.parameter_uses(child),
            }
        }
    }

    fn class(&mut self, node: Node<'_>) {
        self.locals.push(type_parameter_names(node, self.source));
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "class_heritage" => {
                    let mut inner = child.walk();
                    for clause in child.named_children(&mut inner) {
                        match clause.kind() {
                            "extends_clause" => {
                                self.heritage(clause, child.start_byte(), RefKind::Inherit)
                            }
                            "implements_clause" => {
                                self.heritage(clause, child.start_byte(), RefKind::Implement)
                            }
                            // JavaScript: `class A extends B` holds `B` directly.
                            _ => self.base(clause, child.start_byte(), RefKind::Inherit),
                        }
                    }
                }
                "class_body" => self.visit_children(child),
                "decorator" => self.visit(child),
                _ => {}
            }
        }
        self.locals.pop();
    }

    /// The names in an `extends` or `implements` clause.
    fn heritage(&mut self, clause: Node<'_>, class: usize, kind: RefKind) {
        let mut cursor = clause.walk();
        for child in clause.named_children(&mut cursor) {
            if child.kind() == "type_arguments" {
                self.visit(child);
            } else {
                self.base(child, class, kind);
            }
        }
    }

    fn base(&mut self, node: Node<'_>, class: usize, kind: RefKind) {
        match self.path(node) {
            Some(path) => {
                if kind == RefKind::Inherit {
                    match self
                        .facts
                        .bases
                        .iter_mut()
                        .find(|bases| bases.class == class as u32)
                    {
                        Some(bases) => bases.paths.push(path.clone()),
                        None => self.facts.bases.push(Bases {
                            class: class as u32,
                            paths: vec![path.clone()],
                        }),
                    }
                }
                self.add(path, kind);
                // `extends Base<Arg>`: the argument is a use too.
                if node.kind() == "generic_type" {
                    self.visit_field(node, "type_arguments");
                }
            }
            None => self.visit(node),
        }
    }

    /// A dotted path such as `a.b.c`, when the node is one.
    fn path(&self, node: Node<'_>) -> Option<Vec<Seg>> {
        match node.kind() {
            "identifier" | "type_identifier" => Some(vec![Seg::of(node, self.source)]),
            "this" => Some(vec![Seg::marker(SELF, node)]),
            "super" => Some(vec![Seg::marker(SUPER, node)]),
            "generic_type" => self.path(node.child_by_field_name("name")?),
            "non_null_expression" | "parenthesized_expression" => self.path(node.named_child(0)?),
            "member_expression" | "nested_identifier" | "nested_type_identifier" => {
                let object = node
                    .child_by_field_name("object")
                    .or_else(|| node.child_by_field_name("module"))?;
                let property = node
                    .child_by_field_name("property")
                    .or_else(|| node.child_by_field_name("name"))?;
                if !matches!(
                    property.kind(),
                    "property_identifier"
                        | "private_property_identifier"
                        | "type_identifier"
                        | "identifier"
                ) {
                    return None;
                }
                let mut path = self.path(object)?;
                path.push(Seg::of(property, self.source));
                Some(path)
            }
            _ => None,
        }
    }

    fn add(&mut self, path: Vec<Seg>, kind: RefKind) {
        let first = path[0].name.as_str();
        let bare_keyword = (first == SELF || first == SUPER) && path.len() == 1;
        if bare_keyword || self.locals.contains(first) {
            return;
        }
        self.facts.add_use(path, kind);
    }

    #[allow(clippy::too_many_arguments)]
    fn bind(
        &mut self,
        local: Option<String>,
        specifier: String,
        name: Option<String>,
        star: bool,
        reexport: bool,
        statement: Node<'_>,
        place: Node<'_>,
    ) {
        self.facts.bindings.push(Binding {
            local,
            star,
            reexport,
            target: Target::Ecma { specifier, name },
            statement: statement.start_byte() as u32,
            start: place.start_byte() as u32,
            end: place.end_byte() as u32,
        });
    }

    fn import(&mut self, node: Node<'_>) {
        let Some(specifier) = node
            .child_by_field_name("source")
            .and_then(|source| string_value(source, self.source))
        else {
            return;
        };
        let mut cursor = node.walk();
        let clause = node
            .named_children(&mut cursor)
            .find(|child| child.kind() == "import_clause");
        let Some(clause) = clause else {
            // `import "./setup"`
            let place = node.child_by_field_name("source").unwrap_or(node);
            self.bind(None, specifier, None, false, false, node, place);
            return;
        };

        let mut cursor = clause.walk();
        for child in clause.named_children(&mut cursor) {
            match child.kind() {
                // `import name from`
                "identifier" => {
                    let local = text(child, self.source).to_owned();
                    self.bind(
                        Some(local),
                        specifier.clone(),
                        Some(String::from("default")),
                        false,
                        false,
                        node,
                        child,
                    );
                }
                // `import * as name from`
                "namespace_import" => {
                    if let Some(name) = child.named_child(0) {
                        let local = text(name, self.source).to_owned();
                        self.bind(
                            Some(local),
                            specifier.clone(),
                            None,
                            false,
                            false,
                            node,
                            name,
                        );
                    }
                }
                // `import { a, b as c } from`
                "named_imports" => {
                    let mut inner = child.walk();
                    for specifier_node in child.named_children(&mut inner) {
                        let Some(name) = specifier_node.child_by_field_name("name") else {
                            continue;
                        };
                        let local = specifier_node.child_by_field_name("alias").unwrap_or(name);
                        self.bind(
                            Some(text(local, self.source).to_owned()),
                            specifier.clone(),
                            Some(text(name, self.source).to_owned()),
                            false,
                            false,
                            node,
                            local,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    /// `export ... from "module"`: names passed on without being bound here.
    fn reexport(&mut self, node: Node<'_>) {
        let Some(specifier) = node
            .child_by_field_name("source")
            .and_then(|source| string_value(source, self.source))
        else {
            return;
        };
        let mut any = false;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "export_clause" => {
                    any = true;
                    let mut inner = child.walk();
                    for specifier_node in child.named_children(&mut inner) {
                        let Some(name) = specifier_node.child_by_field_name("name") else {
                            continue;
                        };
                        let local = specifier_node.child_by_field_name("alias").unwrap_or(name);
                        self.bind(
                            Some(text(local, self.source).to_owned()),
                            specifier.clone(),
                            Some(text(name, self.source).to_owned()),
                            false,
                            true,
                            node,
                            local,
                        );
                    }
                }
                // `export * as name from`
                "namespace_export" => {
                    any = true;
                    if let Some(name) = child.named_child(0) {
                        let local = text(name, self.source).to_owned();
                        self.bind(
                            Some(local),
                            specifier.clone(),
                            None,
                            false,
                            true,
                            node,
                            name,
                        );
                    }
                }
                _ => {}
            }
        }
        if !any {
            // `export * from`
            let place = node.child_by_field_name("source").unwrap_or(node);
            self.bind(None, specifier, None, true, true, node, place);
        }
    }
}

/// The module a `require("...")` call names, when the argument is a literal.
/// Also sees through `require("...").member`.
fn require_specifier(node: Node<'_>, source: &[u8]) -> Option<String> {
    if node.kind() != "call_expression" {
        return None;
    }
    let function = node.child_by_field_name("function")?;
    if function.kind() != "identifier" || text(function, source) != "require" {
        return None;
    }
    let arguments = node.child_by_field_name("arguments")?;
    if arguments.named_child_count() != 1 {
        return None;
    }
    string_value(arguments.named_child(0)?, source)
}

/// Generic parameter names: `<T, U extends V>`.
fn type_parameter_names(node: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut names = HashSet::new();
    if let Some(parameters) = node.child_by_field_name("type_parameters") {
        let mut cursor = parameters.walk();
        for parameter in parameters.named_children(&mut cursor) {
            if let Some(name) = parameter.child_by_field_name("name") {
                names.insert(text(name, source).to_owned());
            }
        }
    }
    names
}

/// The names a parameter list or a destructuring pattern binds.
fn pattern_names(node: Node<'_>, source: &[u8], out: &mut HashSet<String>) {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => {
            out.insert(text(node, source).to_owned());
        }
        // Types and default values bind nothing.
        "type_annotation" | "decorator" => {}
        "assignment_pattern" | "object_assignment_pattern" => {
            if let Some(left) = node.child_by_field_name("left") {
                pattern_names(left, source, out);
            }
        }
        "pair_pattern" => {
            if let Some(value) = node.child_by_field_name("value") {
                pattern_names(value, source, out);
            }
        }
        "required_parameter" | "optional_parameter" => {
            if let Some(pattern) = node.child_by_field_name("pattern") {
                pattern_names(pattern, source, out);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                pattern_names(child, source, out);
            }
        }
    }
}

/// The names a function body declares with `const`, `let`, `var`, `catch`
/// and `for`. Nested functions and classes keep their own.
fn declared_names(node: Node<'_>, source: &[u8], out: &mut HashSet<String>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let kind = child.kind();
        if is_function(kind) || is_class(kind) {
            continue;
        }
        match kind {
            "variable_declarator" => {
                if let Some(name) = child.child_by_field_name("name") {
                    pattern_names(name, source, out);
                }
            }
            "catch_clause" => {
                if let Some(parameter) = child.child_by_field_name("parameter") {
                    pattern_names(parameter, source, out);
                }
            }
            "for_in_statement" if child.child_by_field_name("kind").is_some() => {
                if let Some(left) = child.child_by_field_name("left") {
                    pattern_names(left, source, out);
                }
            }
            _ => {}
        }
        declared_names(child, source, out);
    }
}
