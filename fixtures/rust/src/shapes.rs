//! Types, traits and impl blocks.

use std::fmt;

use crate::units::Unit;

// construct: trait
pub trait Shape {
    // construct: trait-method
    fn area(&self) -> f64;

    // construct: trait-default-method
    fn describe(&self) -> String {
        format!("shape with area {:.2}", self.area())
    }
}

pub trait Named {
    // construct: assoc-const
    const KIND: &'static str;

    // construct: assoc-type
    type Label: fmt::Display;

    fn label(&self) -> Self::Label;
}

// construct: struct, derive
#[derive(Debug, Clone, PartialEq)]
pub struct Circle {
    // construct: field
    pub radius: f64,
    // construct: field-pub-crate
    pub(crate) unit: Unit,
    // construct: field-private
    id: u32,
}

// construct: tuple-struct
pub struct Meters(pub f64);

// construct: unit-struct
pub struct Origin;

// construct: union
pub union Bits {
    pub int: u32,
    pub float: f32,
}

// construct: enum
pub enum Polygon {
    // construct: variant
    Triangle,
    // construct: variant-tuple
    Regular(u8, f64),
    // construct: variant-struct
    Rect { width: f64, height: f64 },
}

// construct: impl-inherent
impl Circle {
    // construct: constructor
    pub fn new(radius: f64) -> Self {
        Self {
            radius,
            unit: Unit::Meter,
            id: 0,
        }
    }

    // construct: method
    pub fn diameter(&self) -> f64 {
        self.radius * 2.0
    }

    // construct: assoc-fn
    pub fn kind_name() -> &'static str {
        "circle"
    }

    // construct: method-private
    fn bump(&mut self) {
        self.id += 1;
    }
}

// construct: impl-inherent-second
impl Circle {
    // construct: method-returning-self-type
    pub fn scaled(&self, factor: f64) -> Circle {
        Circle {
            radius: self.radius * factor * self.unit.factor(),
            unit: self.unit,
            id: self.id,
        }
    }
}

// construct: impl-trait
impl Shape for Circle {
    fn area(&self) -> f64 {
        std::f64::consts::PI * self.radius * self.radius
    }
}

// construct: impl-trait-assoc-items
impl Named for Circle {
    const KIND: &'static str = "circle";

    type Label = String;

    fn label(&self) -> String {
        format!("{}#{}", Self::KIND, self.id)
    }
}

// construct: impl-trait-generic
impl<T: Shape> Shape for Vec<T> {
    fn area(&self) -> f64 {
        self.iter().map(Shape::area).sum()
    }
}

// construct: impl-trait-std
impl fmt::Display for Polygon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Polygon::Triangle => write!(f, "triangle"),
            Polygon::Regular(sides, _) => write!(f, "{sides}-gon"),
            Polygon::Rect { width, height } => write!(f, "{width}x{height} rect"),
        }
    }
}

// construct: struct-generic
pub struct Tagged<'a, T> {
    pub tag: &'a str,
    pub value: T,
}

// construct: impl-generic
impl<'a, T: Clone> Tagged<'a, T> {
    pub fn value(&self) -> T {
        self.value.clone()
    }
}
