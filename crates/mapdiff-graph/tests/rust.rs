//! Hand-labelled references for Rust.

mod common;

use common::{assert_edges, graph};

const LIB: &str = "\
pub mod shapes;
mod util;

pub use shapes::Circle;
use util::*;

pub const LIMIT: f64 = 10.0;

pub fn total(items: &[Circle]) -> f64 {
    items.iter().map(|item| item.area()).sum::<f64>() + clamp(LIMIT)
}

pub fn build() -> Circle {
    let circle = Circle::new(1.0);
    println!(\"{}\", shapes::describe(&circle));
    circle
}
";

const SHAPES: &str = "\
use std::fmt;

use crate::util::{self, clamp};
use super::LIMIT;

pub trait Shape {
    fn area(&self) -> f64;
    fn double(&self) -> f64 {
        self.area() * 2.0
    }
}

pub struct Circle {
    pub radius: f64,
}

impl Circle {
    pub fn new(radius: f64) -> Self {
        Self { radius: clamp(radius) }
    }

    fn scaled(&self) -> Circle {
        Self::new(self.radius * util::FACTOR)
    }
}

impl Shape for Circle {
    fn area(&self) -> f64 {
        self.radius * self.scaled().radius * LIMIT
    }
}

impl fmt::Display for Circle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, \"{}\", self.radius)
    }
}

pub fn describe(shape: &dyn Shape) -> String {
    format!(\"{}\", shape.area())
}
";

const UTIL: &str = "\
pub const FACTOR: f64 = 2.0;

pub fn clamp(value: f64) -> f64 {
    value.min(super::LIMIT)
}
";

#[test]
fn modules_use_paths_impls_and_macros() {
    let graph = graph(
        "rust",
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"shapes-app\"\nversion = \"0.1.0\"\n",
            ),
            ("src/lib.rs", LIB),
            ("src/shapes.rs", SHAPES),
            ("src/util.rs", UTIL),
        ],
    );
    assert_edges(
        &graph,
        &[
            // `mod name;` lines point at the file that holds the module.
            "src/lib.rs::mod shapes -> src/shapes.rs [import]",
            "src/lib.rs::mod util -> src/util.rs [import]",
            // `use`: a re-export, a glob, `crate::`, `{self, name}`, `super::`.
            "src/lib.rs::{import.1} -> src/shapes.rs::Circle [import]",
            "src/lib.rs::{import.2} -> src/util.rs [import]",
            "src/shapes.rs::{import.2} -> src/util.rs [import]",
            "src/shapes.rs::{import.2} -> src/util.rs::clamp() [import]",
            "src/shapes.rs::{import.3} -> src/lib.rs::LIMIT [import]",
            // A name that arrives through a glob import.
            "src/lib.rs::total() -> src/util.rs::clamp() [call]",
            "src/lib.rs::total() -> src/lib.rs::LIMIT [access]",
            "src/lib.rs::total() -> src/shapes.rs::Circle [type]",
            // An associated function, found in the type's `impl` block.
            "src/lib.rs::build() -> src/shapes.rs::impl Circle::new() [call]",
            "src/lib.rs::build() -> src/shapes.rs::Circle [type]",
            // A path called inside `println!`.
            "src/lib.rs::build() -> src/shapes.rs::describe() [call]",
            // `self.` reaches methods and fields across blocks of one type.
            "src/shapes.rs::Shape::double() -> src/shapes.rs::Shape::area() [call]",
            "src/shapes.rs::impl Circle::scaled() -> src/shapes.rs::Circle::radius [access]",
            "src/shapes.rs::impl Shape for Circle::area() -> src/shapes.rs::Circle::radius [access]",
            "src/shapes.rs::impl Shape for Circle::area() -> src/shapes.rs::impl Circle::scaled() [call]",
            // `Self::`, a constant through a module, a returned type.
            "src/shapes.rs::impl Circle::scaled() -> src/shapes.rs::impl Circle::new() [call]",
            "src/shapes.rs::impl Circle::scaled() -> src/util.rs::FACTOR [access]",
            "src/shapes.rs::impl Circle::scaled() -> src/shapes.rs::Circle [type]",
            "src/shapes.rs::impl Circle::new() -> src/util.rs::clamp() [call]",
            "src/shapes.rs::impl Shape for Circle::area() -> src/lib.rs::LIMIT [access]",
            // An `impl` block names its type and its trait.
            "src/shapes.rs::impl Circle -> src/shapes.rs::Circle [type]",
            "src/shapes.rs::impl Shape for Circle -> src/shapes.rs::Circle [type]",
            "src/shapes.rs::impl Shape for Circle -> src/shapes.rs::Shape [implement]",
            "src/shapes.rs::impl fmt::Display for Circle -> src/shapes.rs::Circle [type]",
            // `self.radius` inside `write!`.
            "src/shapes.rs::impl fmt::Display for Circle::fmt() -> src/shapes.rs::Circle::radius [access]",
            "src/shapes.rs::describe() -> src/shapes.rs::Shape [type]",
            "src/util.rs::clamp() -> src/lib.rs::LIMIT [access]",
        ],
    );
    // Not found, and not expected of the fast tier: `item.area()` in
    // `total` and `shape.area()` in `describe` (a trait method called on a
    // value), and `.radius` on what `self.scaled()` returns.
}
