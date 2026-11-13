//! A private module that lives in its own file.

// construct: enum-fieldless
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unit {
    Meter,
    Foot,
}

impl Unit {
    // construct: const-fn
    pub const fn factor(self) -> f64 {
        match self {
            Unit::Meter => 1.0,
            Unit::Foot => 0.3048,
        }
    }
}
