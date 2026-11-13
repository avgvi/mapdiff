//! Rust: reads uses and `use` declarations off a syntax tree.

use std::collections::HashSet;

use mapdiff_core::RefKind;
use tree_sitter::Node;

use crate::facts::{Binding, Facts, Locals, SELF, SELF_TYPE, Seg, Target, text};

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

    /// Visits every named child except the one in the `name` field.
    fn visit_except_name(&mut self, node: Node<'_>) {
        let name = node.child_by_field_name("name").map(|name| name.id());
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if Some(child.id()) != name {
                self.visit(child);
            }
        }
    }

    fn visit(&mut self, node: Node<'_>) {
        match node.kind() {
            // Nothing in these refers to a scope in the repository, or what
            // they hold is a label rather than a reference.
            "line_comment"
            | "block_comment"
            | "attribute_item"
            | "inner_attribute_item"
            | "string_literal"
            | "raw_string_literal"
            | "char_literal"
            | "integer_literal"
            | "float_literal"
            | "boolean_literal"
            | "lifetime"
            | "primitive_type"
            | "visibility_modifier"
            | "field_identifier"
            | "shorthand_field_identifier"
            | "self"
            | "crate"
            | "super"
            | "mutable_specifier"
            | "self_parameter"
            | "macro_definition"
            | "label" => {}

            "use_declaration" => {
                if let Some(argument) = node.child_by_field_name("argument") {
                    self.use_tree(argument, &[], node);
                }
            }
            "extern_crate_declaration" => {
                if let Some(name) = node.child_by_field_name("name") {
                    let local = node.child_by_field_name("alias").unwrap_or(name);
                    self.bind(
                        Some(text(local, self.source).to_owned()),
                        vec![text(name, self.source).to_owned()],
                        false,
                        node,
                        local,
                    );
                }
            }

            "mod_item" => self.visit_field(node, "body"),

            "function_item" | "function_signature_item" => self.function(node),

            "impl_item" => {
                self.locals.push(type_parameter_names(node, self.source));
                if let Some(of_trait) = node.child_by_field_name("trait") {
                    self.type_path(of_trait, RefKind::Implement);
                }
                if let Some(target) = node.child_by_field_name("type") {
                    self.type_path(target, RefKind::TypeUse);
                }
                self.visit_field(node, "body");
                self.locals.pop();
            }

            "struct_item" | "enum_item" | "union_item" | "trait_item" | "type_item" => {
                self.locals.push(type_parameter_names(node, self.source));
                self.visit_except_name(node);
                self.locals.pop();
            }

            "const_item"
            | "static_item"
            | "field_declaration"
            | "enum_variant"
            | "associated_type"
            | "type_parameter"
            | "constrained_type_parameter" => self.visit_except_name(node),

            "macro_invocation" => {
                if let Some(name) = node.child_by_field_name("macro")
                    && let Some(path) = self.path(name)
                {
                    self.add(path, RefKind::Macro);
                }
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "token_tree" {
                        self.tokens(child);
                    }
                }
            }

            "call_expression" => {
                if let Some(function) = node.child_by_field_name("function") {
                    match self.callee(function) {
                        Some(path) => self.add(path, RefKind::Call),
                        None => self.visit(function),
                    }
                    // `f::<T>()`: the type argument is a use too.
                    if function.kind() == "generic_function" {
                        self.visit_field(function, "type_arguments");
                    }
                }
                self.visit_field(node, "arguments");
            }

            "struct_expression" => {
                if let Some(name) = node.child_by_field_name("name") {
                    self.type_path(name, RefKind::Call);
                }
                self.visit_field(node, "body");
            }
            // `Point { x: value }`: `x` is a label, `value` is a use.
            "field_initializer" => self.visit_field(node, "value"),

            "field_expression" => match self.callee(node) {
                Some(path) => self.add(path, RefKind::Access),
                None => self.visit_field(node, "value"),
            },

            "scoped_identifier" => match self.path(node) {
                Some(path) => self.add(path, RefKind::Access),
                None => self.visit_children(node),
            },
            "scoped_type_identifier" | "generic_type" => self.type_path(node, RefKind::TypeUse),

            "identifier" => self.add(vec![Seg::of(node, self.source)], RefKind::Access),
            "type_identifier" => {
                if text(node, self.source) != "Self" {
                    self.add(vec![Seg::of(node, self.source)], RefKind::TypeUse);
                }
            }

            // Names bound by patterns are not uses, but the types and
            // constants a pattern names are.
            "let_declaration" => {
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    self.pattern(pattern);
                }
                for field in ["type", "value", "alternative"] {
                    self.visit_field(node, field);
                }
            }
            "parameter" => self.visit_field(node, "type"),
            "closure_parameters" => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "parameter" {
                        self.visit_field(child, "type");
                    }
                }
            }
            "for_expression" => {
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    self.pattern(pattern);
                }
                self.visit_field(node, "value");
                self.visit_field(node, "body");
            }
            "match_arm" => {
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    self.pattern(pattern);
                }
                self.visit_field(node, "value");
            }
            "let_condition" => {
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    self.pattern(pattern);
                }
                self.visit_field(node, "value");
            }

            _ => self.visit_children(node),
        }
    }

    fn function(&mut self, node: Node<'_>) {
        let mut names = type_parameter_names(node, self.source);
        bound_names(node, self.source, &mut names, true);
        self.locals.push(names);
        self.visit_except_name(node);
        self.locals.pop();
    }

    /// A pattern: lower-case names are bound, everything else is a use.
    fn pattern(&mut self, node: Node<'_>) {
        match node.kind() {
            "identifier" => {
                // By convention a capitalised name in a pattern is a
                // constant or a unit variant, not a new binding.
                if starts_uppercase(text(node, self.source)) {
                    self.add(vec![Seg::of(node, self.source)], RefKind::Access);
                }
            }
            "scoped_identifier" => {
                if let Some(path) = self.path(node) {
                    self.add(path, RefKind::Access);
                }
            }
            "tuple_struct_pattern" | "struct_pattern" => {
                let named = node.child_by_field_name("type");
                if let Some(named) = named {
                    self.type_path(named, RefKind::Access);
                }
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if Some(child.id()) != named.map(|named| named.id()) {
                        self.pattern(child);
                    }
                }
            }
            // `Point { x: inner }`: `x` is a label, `inner` is a pattern.
            "field_pattern" => {
                if let Some(inner) = node.child_by_field_name("pattern") {
                    self.pattern(inner);
                }
            }
            "field_identifier" | "shorthand_field_identifier" | "mutable_specifier" => {}
            // Literals and ranges in patterns can name constants.
            "integer_literal" | "string_literal" | "char_literal" | "boolean_literal"
            | "float_literal" => {}
            _ => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    self.pattern(child);
                }
            }
        }
    }

    /// A type written as a path, with its generic arguments visited too.
    fn type_path(&mut self, node: Node<'_>, kind: RefKind) {
        match self.path(node) {
            Some(path) => self.add(path, kind),
            None => {
                self.visit_children(node);
                return;
            }
        }
        if node.kind() == "generic_type" {
            self.visit_field(node, "type_arguments");
        }
    }

    /// What a call or field access starts from: a path, or `self.name`.
    fn callee(&self, node: Node<'_>) -> Option<Vec<Seg>> {
        match node.kind() {
            "field_expression" => {
                let value = node.child_by_field_name("value")?;
                let field = node.child_by_field_name("field")?;
                (value.kind() == "self" && field.kind() == "field_identifier")
                    .then(|| vec![Seg::marker(SELF, value), Seg::of(field, self.source)])
            }
            "generic_function" => self.callee(node.child_by_field_name("function")?),
            _ => self.path(node),
        }
    }

    /// A `::`-separated path, when the node is one.
    fn path(&self, node: Node<'_>) -> Option<Vec<Seg>> {
        match node.kind() {
            "identifier" | "type_identifier" => Some(vec![if text(node, self.source) == "Self" {
                Seg::marker(SELF_TYPE, node)
            } else {
                Seg::of(node, self.source)
            }]),
            // In a path these are module keywords, kept under their own names.
            "self" | "crate" | "super" => Some(vec![Seg::of(node, self.source)]),
            "generic_type" | "generic_type_with_turbofish" => {
                self.path(node.child_by_field_name("type")?)
            }
            "scoped_identifier" | "scoped_type_identifier" => {
                let name = node.child_by_field_name("name")?;
                // Without a prefix, `::name` starts at the list of crates:
                // nothing local.
                let mut path = self.path(node.child_by_field_name("path")?)?;
                path.push(Seg::of(name, self.source));
                Some(path)
            }
            _ => None,
        }
    }

    fn add(&mut self, path: Vec<Seg>, kind: RefKind) {
        let first = path[0].name.as_str();
        let bare_keyword =
            path.len() == 1 && matches!(first, SELF | SELF_TYPE | "self" | "crate" | "super");
        // Only a bare name can be a local; `a::b` always starts at an item.
        if bare_keyword || (path.len() == 1 && self.locals.contains(first)) {
            return;
        }
        self.facts.add_use(path, kind);
    }

    /// The inside of a macro call is unparsed tokens. Paths, calls and
    /// `self.name` are still recognisable by shape, and are too common in
    /// `format!`, `assert!` and `vec!` to ignore.
    fn tokens(&mut self, tree: Node<'_>) {
        let mut cursor = tree.walk();
        let tokens: Vec<Node<'_>> = tree.children(&mut cursor).collect();
        let mut position = 0;
        while position < tokens.len() {
            let token = tokens[position];
            match token.kind() {
                "token_tree" => {
                    self.tokens(token);
                    position += 1;
                }
                "identifier" | "self" | "crate" | "super" => {
                    let previous = position.checked_sub(1).map(|before| tokens[before].kind());
                    let mut path = vec![token];
                    while tokens.get(position + 1).map(|next| next.kind()) == Some("::")
                        && tokens
                            .get(position + 2)
                            .is_some_and(|next| matches!(next.kind(), "identifier" | "super"))
                    {
                        path.push(tokens[position + 2]);
                        position += 2;
                    }
                    // `self.name`: a member of the enclosing type.
                    let mut segments: Vec<Seg> = if path.len() == 1
                        && token.kind() == "self"
                        && tokens.get(position + 1).map(|next| next.kind()) == Some(".")
                        && tokens.get(position + 2).map(|next| next.kind()) == Some("identifier")
                    {
                        position += 2;
                        vec![
                            Seg::marker(SELF, token),
                            Seg::of(tokens[position], self.source),
                        ]
                    } else {
                        path.iter()
                            .map(|part| Seg::of(*part, self.source))
                            .collect()
                    };
                    if segments[0].name == "Self" {
                        segments[0] = Seg::marker(SELF_TYPE, path[0]);
                    }
                    let next = tokens.get(position + 1);
                    let kind = match next.map(|next| next.kind()) {
                        Some("token_tree")
                            if text(next.copied().unwrap_or(token), self.source)
                                .starts_with('(') =>
                        {
                            RefKind::Call
                        }
                        Some("!") => RefKind::Macro,
                        _ => RefKind::Access,
                    };
                    // After a dot the name is a field or method of a value,
                    // which cannot be resolved from tokens.
                    if previous != Some(".") {
                        self.add(segments, kind);
                    }
                    position += 1;
                }
                _ => position += 1,
            }
        }
    }

    fn use_tree(&mut self, node: Node<'_>, prefix: &[String], statement: Node<'_>) {
        let names = |node: Node<'_>| -> Vec<String> {
            self.path(node)
                .map(|path| path.into_iter().map(|segment| segment.name).collect())
                .unwrap_or_default()
        };
        let joined =
            |more: Vec<String>| -> Vec<String> { prefix.iter().cloned().chain(more).collect() };

        match node.kind() {
            // `use a::{self}` binds the last name of the prefix.
            "self" if !prefix.is_empty() => {
                let local = prefix.last().cloned();
                self.bind(local, prefix.to_vec(), false, statement, node);
            }
            "identifier" | "scoped_identifier" | "crate" | "super" | "self" => {
                let full = joined(names(node));
                let local = full.last().cloned();
                self.bind(local, full, false, statement, last_name(node));
            }
            "use_as_clause" => {
                let (Some(path), Some(alias)) = (
                    node.child_by_field_name("path"),
                    node.child_by_field_name("alias"),
                ) else {
                    return;
                };
                let local = text(alias, self.source).to_owned();
                self.bind(Some(local), joined(names(path)), false, statement, alias);
            }
            "scoped_use_list" => {
                let inner = match node.child_by_field_name("path") {
                    Some(path) => joined(names(path)),
                    None => prefix.to_vec(),
                };
                if let Some(list) = node.child_by_field_name("list") {
                    self.use_tree(list, &inner, statement);
                }
            }
            "use_list" => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    self.use_tree(child, prefix, statement);
                }
            }
            "use_wildcard" => {
                let full = match node.named_child(0) {
                    Some(path) => joined(names(path)),
                    None => prefix.to_vec(),
                };
                let place = node.named_child(0).map_or(node, last_name);
                self.bind(None, full, true, statement, place);
            }
            _ => {}
        }
    }

    fn bind(
        &mut self,
        local: Option<String>,
        path: Vec<String>,
        star: bool,
        statement: Node<'_>,
        place: Node<'_>,
    ) {
        if path.is_empty() {
            return;
        }
        self.facts.bindings.push(Binding {
            local,
            star,
            reexport: false,
            target: Target::Rust { path },
            statement: statement.start_byte() as u32,
            start: place.start_byte() as u32,
            end: place.end_byte() as u32,
        });
    }
}

/// The last name of a `::` path: where the thing a `use` brings in is written.
fn last_name(node: Node<'_>) -> Node<'_> {
    match node.kind() {
        "scoped_identifier" | "scoped_type_identifier" => {
            node.child_by_field_name("name").unwrap_or(node)
        }
        _ => node,
    }
}

fn starts_uppercase(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}

/// Generic parameter names: `<T, U: Bound>`.
fn type_parameter_names(node: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut names = HashSet::new();
    if let Some(parameters) = node.child_by_field_name("type_parameters") {
        let mut cursor = parameters.walk();
        for parameter in parameters.named_children(&mut cursor) {
            let name = match parameter.kind() {
                "type_identifier" => Some(parameter),
                "lifetime" | "lifetime_parameter" => None,
                // `T: Bound`, `T = Default`, `const N: usize`
                _ => parameter
                    .child_by_field_name("name")
                    .or_else(|| parameter.child_by_field_name("left"))
                    .or_else(|| parameter.named_child(0)),
            };
            if let Some(name) = name {
                names.insert(text(name, source).to_owned());
            }
        }
    }
    names
}

/// Every name a function binds: parameters, `let`, closures, loops and
/// match arms. Nested functions keep their own.
fn bound_names(node: Node<'_>, source: &[u8], out: &mut HashSet<String>, top: bool) {
    if !top && matches!(node.kind(), "function_item" | "token_tree") {
        return;
    }
    let pattern = match node.kind() {
        "parameter" | "let_declaration" | "for_expression" | "match_arm" | "let_condition" => {
            node.child_by_field_name("pattern")
        }
        _ => None,
    };
    if let Some(pattern) = pattern {
        pattern_bindings(pattern, source, out);
    }
    if node.kind() == "closure_parameters" {
        // `|a, b|` lists bare names; `|a: T|` wraps them in parameters.
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() != "parameter" {
                pattern_bindings(child, source, out);
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        bound_names(child, source, out, false);
    }
}

fn pattern_bindings(node: Node<'_>, source: &[u8], out: &mut HashSet<String>) {
    match node.kind() {
        "identifier" | "shorthand_field_identifier" => {
            let name = text(node, source);
            if !starts_uppercase(name) {
                out.insert(name.to_owned());
            }
        }
        // A path in a pattern names something; it binds nothing.
        "scoped_identifier" | "scoped_type_identifier" | "type_identifier" | "field_identifier" => {
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                pattern_bindings(child, source, out);
            }
        }
    }
}
