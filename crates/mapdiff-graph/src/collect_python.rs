//! Python: reads uses, imports and base classes off a syntax tree.

use std::collections::HashSet;

use mapdiff_core::RefKind;
use tree_sitter::Node;

use crate::facts::{Bases, Binding, Facts, Locals, SELF, SUPER, Seg, Target, text};

pub fn collect(root: Node<'_>, source: &[u8]) -> Facts {
    let mut collector = Collector {
        source,
        facts: Facts::default(),
        locals: Locals::default(),
        receivers: Vec::new(),
    };
    collector.visit(root, false);
    collector.facts
}

struct Collector<'s> {
    source: &'s [u8],
    facts: Facts,
    locals: Locals,
    /// The name of the first parameter of each enclosing method (`self`,
    /// `cls`), innermost last. `None` for a plain function.
    receivers: Vec<Option<String>>,
}

impl Collector<'_> {
    fn visit_children(&mut self, node: Node<'_>, in_type: bool) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.visit(child, in_type);
        }
    }

    fn visit(&mut self, node: Node<'_>, in_type: bool) {
        let field = |name: &str| node.child_by_field_name(name);
        match node.kind() {
            "comment" | "future_import_statement" | "global_statement" | "nonlocal_statement" => {}

            // Only the interpolated parts of a string hold code.
            "string" => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "interpolation" {
                        self.visit_children(child, false);
                    }
                }
            }

            "import_statement" => self.import(node),
            "import_from_statement" => self.import_from(node),

            "function_definition" => self.function(node),
            "lambda" => {
                let mut names = HashSet::new();
                if let Some(parameters) = field("parameters") {
                    bound_names(parameters, self.source, &mut names);
                }
                self.locals.push(names);
                if let Some(body) = field("body") {
                    self.visit(body, false);
                }
                self.locals.pop();
            }

            "class_definition" => {
                if let Some(bases) = field("superclasses") {
                    let mut paths = Vec::new();
                    let mut cursor = bases.walk();
                    for base in bases.named_children(&mut cursor) {
                        match self.path(base) {
                            Some(path) => {
                                paths.push(path.clone());
                                self.add(path, RefKind::Inherit);
                            }
                            // `metaclass=Meta` and other expressions.
                            None => self.visit(base, false),
                        }
                    }
                    self.facts.bases.push(Bases {
                        class: bases.start_byte() as u32,
                        paths,
                    });
                }
                if let Some(body) = field("body") {
                    self.visit(body, false);
                }
            }

            "call" => {
                if let Some(function) = field("function") {
                    match self.path(function) {
                        Some(path) => self.add(path, RefKind::Call),
                        None => self.visit(function, false),
                    }
                }
                if let Some(arguments) = field("arguments") {
                    self.visit(arguments, false);
                }
            }

            "attribute" => match self.path(node) {
                Some(path) => self.add(path, kind_for(in_type)),
                // The chain starts at something that is not a name, such as
                // a call: only that inner part can hold uses.
                None => {
                    if let Some(object) = field("object") {
                        self.visit(object, in_type);
                    }
                }
            },

            "identifier" => {
                let seg = Seg::of(node, self.source);
                self.add(vec![seg], kind_for(in_type));
            }

            "type" => self.visit_children(node, true),

            "keyword_argument" => {
                if let Some(value) = field("value") {
                    self.visit(value, in_type);
                }
            }

            // The names an assignment or loop binds are not uses.
            "assignment" | "augmented_assignment" | "named_expression" => {
                let target = field("left").or_else(|| field("name"));
                if let Some(target) = target {
                    self.visit_target(target);
                }
                for part in ["type", "right", "value"] {
                    if let Some(child) = field(part) {
                        self.visit(child, part == "type");
                    }
                }
            }
            "for_statement" | "for_in_clause" => {
                if let Some(target) = field("left") {
                    self.visit_target(target);
                }
                let mut cursor = node.walk();
                for (position, child) in node.named_children(&mut cursor).enumerate() {
                    if position > 0 || field("left").is_none() {
                        self.visit(child, false);
                    }
                }
            }
            "as_pattern" => {
                // `with open(x) as f`, `except E as e`: the first child is used.
                if let Some(value) = node.named_child(0) {
                    self.visit(value, false);
                }
            }

            "parameters" | "lambda_parameters" => self.parameters(node),

            _ => self.visit_children(node, in_type),
        }
    }

    /// The left side of an assignment: plain names are bound, anything else
    /// (`self.x`, `items[i]`) is used.
    fn visit_target(&mut self, target: Node<'_>) {
        match target.kind() {
            "identifier" => {}
            "pattern_list" | "tuple_pattern" | "list_pattern" | "list_splat_pattern" => {
                let mut cursor = target.walk();
                for child in target.named_children(&mut cursor) {
                    self.visit_target(child);
                }
            }
            _ => self.visit(target, false),
        }
    }

    /// Parameter names are bound; their annotations and defaults are uses.
    fn parameters(&mut self, parameters: Node<'_>) {
        let mut cursor = parameters.walk();
        for parameter in parameters.named_children(&mut cursor) {
            if let Some(annotation) = parameter.child_by_field_name("type") {
                self.visit(annotation, true);
            }
            if let Some(default) = parameter.child_by_field_name("value") {
                self.visit(default, false);
            }
        }
    }

    fn function(&mut self, node: Node<'_>) {
        let mut names = HashSet::new();
        let parameters = node.child_by_field_name("parameters");
        if let Some(parameters) = parameters {
            bound_names(parameters, self.source, &mut names);
        }
        let body = node.child_by_field_name("body");
        if let Some(body) = body {
            assigned_names(body, self.source, &mut names);
        }

        // A function defined directly in a class body is a method, and its
        // first parameter is the object (or class) it is called on.
        let is_method = {
            let mut parent = node.parent();
            if parent.is_some_and(|parent| parent.kind() == "decorated_definition") {
                parent = parent.and_then(|parent| parent.parent());
            }
            parent
                .filter(|block| block.kind() == "block")
                .and_then(|block| block.parent())
                .is_some_and(|class| class.kind() == "class_definition")
        };
        let receiver = if is_method {
            parameters
                .and_then(|parameters| parameters.named_child(0))
                .filter(|first| first.kind() == "identifier")
                .map(|first| text(first, self.source).to_owned())
        } else {
            None
        };
        // The receiver is resolved as `<self>`, so it must not count as a local.
        if let Some(receiver) = &receiver {
            names.remove(receiver);
        }

        if let Some(parameters) = parameters {
            self.parameters(parameters);
        }
        if let Some(return_type) = node.child_by_field_name("return_type") {
            self.visit(return_type, true);
        }
        self.locals.push(names);
        // A nested plain function still sees the method's `self`.
        self.receivers
            .push(receiver.or_else(|| self.receivers.last().cloned().flatten()));
        if let Some(body) = body {
            self.visit(body, false);
        }
        self.receivers.pop();
        self.locals.pop();
    }

    /// A dotted path such as `a.b.c`, when the node is one. The receiver of
    /// a method becomes `<self>`, and `super()` becomes `<super>`.
    fn path(&self, node: Node<'_>) -> Option<Vec<Seg>> {
        match node.kind() {
            "identifier" => {
                let name = text(node, self.source);
                let is_receiver =
                    self.receivers.last().and_then(|name| name.as_deref()) == Some(name);
                Some(vec![if is_receiver {
                    Seg::marker(SELF, node)
                } else {
                    Seg::of(node, self.source)
                }])
            }
            "attribute" => {
                let object = node.child_by_field_name("object")?;
                let attribute = node.child_by_field_name("attribute")?;
                let mut path = if is_super_call(object, self.source) {
                    vec![Seg::marker(SUPER, object)]
                } else {
                    self.path(object)?
                };
                path.push(Seg::of(attribute, self.source));
                Some(path)
            }
            _ => None,
        }
    }

    fn add(&mut self, path: Vec<Seg>, kind: RefKind) {
        let first = path[0].name.as_str();
        // A bare `self` says nothing, and local names are not references.
        if (first == SELF && path.len() == 1) || self.locals.contains(first) {
            return;
        }
        self.facts.add_use(path, kind);
    }

    fn import(&mut self, node: Node<'_>) {
        let statement = node.start_byte() as u32;
        let mut cursor = node.walk();
        for child in node.children_by_field_name("name", &mut cursor) {
            let (module, alias) = match child.kind() {
                "aliased_import" => (
                    child.child_by_field_name("name"),
                    child.child_by_field_name("alias"),
                ),
                _ => (Some(child), None),
            };
            let Some(module) = module else { continue };
            let parts = dotted(module, self.source);
            if parts.is_empty() {
                continue;
            }
            let place = alias.unwrap_or(module);
            match alias {
                // `import a.b as m` binds `m` to the module `a.b`.
                Some(alias) => self.bind(
                    Some(text(alias, self.source)),
                    parts,
                    None,
                    false,
                    statement,
                    place,
                ),
                None => {
                    // `import a.b` binds only `a`, but depends on `a.b`.
                    if parts.len() > 1 {
                        self.bind(None, parts.clone(), None, false, statement, place);
                    }
                    let first = parts[0].clone();
                    self.bind(
                        Some(&first.clone()),
                        vec![first],
                        None,
                        false,
                        statement,
                        place,
                    );
                }
            }
        }
    }

    fn import_from(&mut self, node: Node<'_>) {
        let statement = node.start_byte() as u32;
        let Some(module) = node.child_by_field_name("module_name") else {
            return;
        };
        let (level, parts) = match module.kind() {
            "relative_import" => {
                let mut level = 0;
                let mut parts = Vec::new();
                let mut cursor = module.walk();
                for child in module.named_children(&mut cursor) {
                    match child.kind() {
                        "import_prefix" => {
                            level = text(child, self.source).matches('.').count() as u32
                        }
                        "dotted_name" => parts = dotted(child, self.source),
                        _ => {}
                    }
                }
                (level, parts)
            }
            _ => (0, dotted(module, self.source)),
        };

        let mut cursor = node.walk();
        let mut any = false;
        for child in node.children_by_field_name("name", &mut cursor) {
            any = true;
            let (name, alias) = match child.kind() {
                "aliased_import" => (
                    child.child_by_field_name("name"),
                    child.child_by_field_name("alias"),
                ),
                _ => (Some(child), None),
            };
            let Some(name) = name else { continue };
            let local = alias.unwrap_or(name);
            self.facts.bindings.push(Binding {
                local: Some(text(local, self.source).to_owned()),
                star: false,
                reexport: false,
                target: Target::Python {
                    level,
                    module: parts.clone(),
                    name: Some(text(name, self.source).to_owned()),
                },
                statement,
                start: local.start_byte() as u32,
                end: local.end_byte() as u32,
            });
        }
        if !any {
            // `from module import *`
            self.facts.bindings.push(Binding {
                local: None,
                star: true,
                reexport: false,
                target: Target::Python {
                    level,
                    module: parts,
                    name: None,
                },
                statement,
                start: module.start_byte() as u32,
                end: module.end_byte() as u32,
            });
        }
    }

    fn bind(
        &mut self,
        local: Option<&str>,
        module: Vec<String>,
        name: Option<String>,
        star: bool,
        statement: u32,
        place: Node<'_>,
    ) {
        self.facts.bindings.push(Binding {
            local: local.map(str::to_owned),
            star,
            reexport: false,
            target: Target::Python {
                level: 0,
                module,
                name,
            },
            statement,
            start: place.start_byte() as u32,
            end: place.end_byte() as u32,
        });
    }
}

fn kind_for(in_type: bool) -> RefKind {
    if in_type {
        RefKind::TypeUse
    } else {
        RefKind::Access
    }
}

fn dotted(node: Node<'_>, source: &[u8]) -> Vec<String> {
    text(node, source)
        .split('.')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn is_super_call(node: Node<'_>, source: &[u8]) -> bool {
    node.kind() == "call"
        && node
            .child_by_field_name("function")
            .is_some_and(|function| {
                function.kind() == "identifier" && text(function, source) == "super"
            })
}

/// The names a parameter list binds.
fn bound_names(parameters: Node<'_>, source: &[u8], out: &mut HashSet<String>) {
    let mut cursor = parameters.walk();
    for parameter in parameters.named_children(&mut cursor) {
        let name = match parameter.kind() {
            "identifier" => Some(parameter),
            "default_parameter" | "typed_default_parameter" => {
                parameter.child_by_field_name("name")
            }
            // `x: int`, `*args`, `**kwargs`: the name is the first identifier.
            _ => first_identifier(parameter),
        };
        if let Some(name) = name {
            out.insert(text(name, source).to_owned());
        }
    }
}

fn first_identifier(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() == "identifier" {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| child.kind() != "type")
        .find_map(first_identifier)
}

/// The names a function body binds by assignment, loops, `with` and
/// `except`. Nested functions and classes keep their own.
fn assigned_names(node: Node<'_>, source: &[u8], out: &mut HashSet<String>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_definition" | "class_definition" | "lambda" => continue,
            "assignment"
            | "augmented_assignment"
            | "named_expression"
            | "for_statement"
            | "for_in_clause" => {
                let target = child
                    .child_by_field_name("left")
                    .or_else(|| child.child_by_field_name("name"));
                if let Some(target) = target {
                    pattern_names(target, source, out);
                }
            }
            "as_pattern" => {
                if let Some(alias) = child.child_by_field_name("alias") {
                    pattern_names(alias, source, out);
                }
            }
            _ => {}
        }
        assigned_names(child, source, out);
    }
}

fn pattern_names(node: Node<'_>, source: &[u8], out: &mut HashSet<String>) {
    match node.kind() {
        "identifier" => {
            out.insert(text(node, source).to_owned());
        }
        "pattern_list" | "tuple_pattern" | "list_pattern" | "list_splat_pattern"
        | "as_pattern_target" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                pattern_names(child, source, out);
            }
        }
        _ => {}
    }
}
