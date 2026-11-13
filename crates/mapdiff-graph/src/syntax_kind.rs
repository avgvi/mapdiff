//! Reads how a name is used off the syntax around it: called, used as a
//! type, extended, and so on. The precise tier knows what a name refers to
//! but not how it is used, so this fills in the kind of each reference.

use mapdiff_core::RefKind;
use tree_sitter::Node;

/// Parents in which the last part of a dotted path stands for the whole
/// path: `a.b.c` is called when `c` is.
const PATH_PARENTS: [&str; 10] = [
    "member_expression",
    "attribute",
    "field_expression",
    "scoped_identifier",
    "scoped_type_identifier",
    "nested_type_identifier",
    "nested_identifier",
    "generic_function",
    "generic_type",
    "non_null_expression",
];

/// The field that holds the last part of each path-like parent.
const LAST_PART_FIELDS: [&str; 6] = ["property", "attribute", "field", "name", "function", "type"];

/// Node kinds that mean "everything below is a type".
const TYPE_CONTEXTS: [&str; 16] = [
    "type",
    "type_annotation",
    "type_arguments",
    "type_parameters",
    "generic_type",
    "reference_type",
    "dynamic_type",
    "array_type",
    "tuple_type",
    "union_type",
    "pointer_type",
    "function_type",
    "abstract_type",
    "bounded_type",
    "trait_bounds",
    "where_predicate",
];

/// How the name at `[start, end)` is used.
pub fn kind_at(root: Node<'_>, start: usize, end: usize) -> RefKind {
    let Some(mut node) = root.descendant_for_byte_range(start, end.max(start)) else {
        return RefKind::Access;
    };
    let is_type_name = matches!(node.kind(), "type_identifier" | "scoped_type_identifier");

    // Climb while the name is the last part of a dotted path.
    while let Some(parent) = node.parent() {
        if !PATH_PARENTS.contains(&parent.kind()) {
            break;
        }
        let is_last = LAST_PART_FIELDS
            .iter()
            .filter_map(|field| parent.child_by_field_name(field))
            .any(|part| part.id() == node.id());
        if !is_last {
            // A qualifier such as `module` in `module.name`.
            return if is_type_name {
                RefKind::TypeUse
            } else {
                RefKind::Access
            };
        }
        node = parent;
    }

    let Some(parent) = node.parent() else {
        return RefKind::Access;
    };
    let is_field = |field: &str| {
        parent
            .child_by_field_name(field)
            .is_some_and(|child| child.id() == node.id())
    };
    match parent.kind() {
        "call_expression" | "call" if is_field("function") => return RefKind::Call,
        "new_expression" if is_field("constructor") => return RefKind::Call,
        "struct_expression" if is_field("name") => return RefKind::Call,
        "jsx_opening_element" | "jsx_self_closing_element" if is_field("name") => {
            return RefKind::Call;
        }
        "macro_invocation" if is_field("macro") => return RefKind::Macro,
        "impl_item" if is_field("trait") => return RefKind::Implement,
        "implements_clause" => return RefKind::Implement,
        "extends_clause" | "extends_type_clause" | "class_heritage" => return RefKind::Inherit,
        // Python: `class A(Base)`.
        "argument_list"
            if parent
                .parent()
                .is_some_and(|class| class.kind() == "class_definition") =>
        {
            return RefKind::Inherit;
        }
        _ => {}
    }

    if is_type_name || in_type_context(node) {
        RefKind::TypeUse
    } else {
        RefKind::Access
    }
}

/// True when a few steps up there is a node that only holds types, with no
/// expression in between.
fn in_type_context(node: Node<'_>) -> bool {
    let mut current = node.parent();
    for _ in 0..8 {
        let Some(ancestor) = current else {
            return false;
        };
        let kind = ancestor.kind();
        if TYPE_CONTEXTS.contains(&kind) {
            return true;
        }
        if kind.ends_with("expression")
            || kind.ends_with("statement")
            || kind == "block"
            || kind == "call"
        {
            return false;
        }
        current = ancestor.parent();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use mapdiff_extract::Grammar;

    /// The kind of the first occurrence of `name` in `source`.
    fn kind_of(grammar: Grammar, source: &str, name: &str) -> RefKind {
        let tree = mapdiff_extract::parse::parse(grammar, source.as_bytes()).unwrap();
        let start = source
            .find(name)
            .unwrap_or_else(|| panic!("{name} is in the source"));
        kind_at(tree.root_node(), start, start + name.len())
    }

    #[test]
    fn calls_types_and_plain_uses() {
        let rust = "fn f(s: &Shape) -> u8 { helper(s.area(), LIMIT) + Circle::new(1).r + m!(1) }";
        assert_eq!(kind_of(Grammar::Rust, rust, "Shape"), RefKind::TypeUse);
        assert_eq!(kind_of(Grammar::Rust, rust, "helper"), RefKind::Call);
        assert_eq!(kind_of(Grammar::Rust, rust, "area"), RefKind::Call);
        assert_eq!(kind_of(Grammar::Rust, rust, "LIMIT"), RefKind::Access);
        assert_eq!(kind_of(Grammar::Rust, rust, "new"), RefKind::Call);
        assert_eq!(kind_of(Grammar::Rust, rust, "Circle"), RefKind::Access);
        assert_eq!(kind_of(Grammar::Rust, rust, "m!"), RefKind::Access);
        assert_eq!(kind_of(Grammar::Rust, rust, "m"), RefKind::Macro);

        let python = "def f(shape: Shape):\n    return util.helper(shape.area(), LIMIT)\n";
        assert_eq!(kind_of(Grammar::Python, python, "Shape"), RefKind::TypeUse);
        assert_eq!(kind_of(Grammar::Python, python, "helper"), RefKind::Call);
        assert_eq!(kind_of(Grammar::Python, python, "util"), RefKind::Access);
        assert_eq!(kind_of(Grammar::Python, python, "area"), RefKind::Call);
        assert_eq!(kind_of(Grammar::Python, python, "LIMIT"), RefKind::Access);

        let typescript =
            "function f(shape: Shape) { return new Circle(shape.area(), util.LIMIT); }";
        assert_eq!(
            kind_of(Grammar::TypeScript, typescript, "Shape"),
            RefKind::TypeUse
        );
        assert_eq!(
            kind_of(Grammar::TypeScript, typescript, "Circle"),
            RefKind::Call
        );
        assert_eq!(
            kind_of(Grammar::TypeScript, typescript, "area"),
            RefKind::Call
        );
        assert_eq!(
            kind_of(Grammar::TypeScript, typescript, "LIMIT"),
            RefKind::Access
        );
    }

    #[test]
    fn class_relations() {
        let rust = "impl Shape for Circle {}";
        assert_eq!(kind_of(Grammar::Rust, rust, "Shape"), RefKind::Implement);
        assert_eq!(kind_of(Grammar::Rust, rust, "Circle"), RefKind::TypeUse);

        assert_eq!(
            kind_of(Grammar::Python, "class Circle(Shape):\n    pass\n", "Shape"),
            RefKind::Inherit
        );

        let typescript = "class Circle extends Base implements Shape {}";
        assert_eq!(
            kind_of(Grammar::TypeScript, typescript, "Base"),
            RefKind::Inherit
        );
        assert_eq!(
            kind_of(Grammar::TypeScript, typescript, "Shape"),
            RefKind::Implement
        );
        assert_eq!(
            kind_of(
                Grammar::JavaScript,
                "class App extends Emitter {}",
                "Emitter"
            ),
            RefKind::Inherit
        );
        assert_eq!(
            kind_of(Grammar::Tsx, "const a = <Badge shape={s} />;", "Badge"),
            RefKind::Call
        );
    }
}
