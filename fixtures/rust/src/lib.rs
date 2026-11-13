//! Hand-made fixture. Every Rust construct the extractor must map appears
//! here or in a sibling module, tagged with a `construct:` marker on the line
//! above it. The list of required markers is `fixtures/constructs/rust.txt`.
#![allow(dead_code, unused)]

// construct: mod-file
pub mod shapes;
// construct: mod-file-private
mod units;

// construct: use
use std::collections::HashMap;
// construct: use-group
use std::fmt::{self, Display};
// construct: use-glob
use std::cmp::*;
// construct: pub-use
pub use shapes::Shape;

// construct: const, access-pub
pub const MAX_SHAPES: usize = 64;

// construct: static, access-private
static REGISTRY_NAME: &str = "default";

// construct: type-alias
pub type ShapeMap = HashMap<String, shapes::Circle>;

// construct: macro-rules
macro_rules! make_id {
    ($name:ident) => {
        pub struct $name(pub u32);
    };
}

// construct: macro-made-item
make_id!(ShapeId);

// construct: fn, doc-comment
/// Sums the area of every shape.
///
/// The doc comment belongs to the function below it.
pub fn total_area(shapes: &[&dyn Shape]) -> f64 {
    // construct: closure
    let area_of = |shape: &dyn Shape| shape.area();
    shapes.iter().map(|shape| area_of(*shape)).sum()
}

// construct: fn-generic, access-pub-crate
pub(crate) fn largest<T: PartialOrd + Copy>(items: &[T]) -> Option<T> {
    let mut best = *items.first()?;
    for &item in items {
        if item > best {
            best = item;
        }
    }
    Some(best)
}

// construct: async-fn
pub async fn load_count() -> usize {
    MAX_SHAPES
}

// construct: attribute
#[inline]
#[must_use]
pub fn registry_name() -> &'static str {
    REGISTRY_NAME
}

// construct: mod-inline
mod registry {
    use super::ShapeMap;

    // construct: access-pub-super
    pub(super) fn empty() -> ShapeMap {
        ShapeMap::new()
    }

    // construct: mod-nested
    pub mod nested {
        pub fn depth() -> u8 {
            2
        }
    }
}

// construct: cfg-test-mod
#[cfg(test)]
mod tests {
    use super::*;

    // construct: test-fn
    #[test]
    fn largest_of_empty_is_none() {
        assert_eq!(largest::<u8>(&[]), None);
    }
}
