//! Hand-labelled references for Python.

mod common;

use common::{assert_edges, graph};

const MODELS: &str = "\
import math
from .util import clamp as limit, helper
from . import util


class Shape:
    def area(self):
        return 0

    def describe(self):
        return f\"{self.area()}\"


class Circle(Shape):
    scale = 2

    def __init__(self, radius):
        self.radius = limit(radius, 0, 10)

    def area(self):
        return math.pi * self.radius ** 2 * self.scale

    def twice(self):
        return super().describe() + util.helper(self.area())


def make(radius):
    circle = Circle(radius)
    return circle.area()
";

const UTIL: &str = "\
import functools


def logged(func):
    @functools.wraps(func)
    def wrapper(*args):
        return func(*args)

    return wrapper


@logged
def clamp(value, low, high):
    return max(low, min(high, value))


def helper(value):
    return clamp(value, 0, 1)
";

const MAIN: &str = "\
import pkg
import pkg.util as u
from pkg import Circle


def run():
    c = Circle(1)
    pkg.models.make(2)
    return u.helper(3)
";

#[test]
fn imports_methods_inheritance_and_packages() {
    let graph = graph(
        "python",
        &[
            ("pkg/__init__.py", "from .models import Circle\n"),
            ("pkg/models.py", MODELS),
            ("pkg/util.py", UTIL),
            ("main.py", MAIN),
        ],
    );
    assert_edges(
        &graph,
        &[
            // Imports, including an alias and a package re-export.
            "pkg/__init__.py::{import.1} -> pkg/models.py::Circle [import]",
            "pkg/models.py::{import.2} -> pkg/util.py::clamp() [import]",
            "pkg/models.py::{import.2} -> pkg/util.py::helper() [import]",
            "pkg/models.py::{import.3} -> pkg/util.py [import]",
            "main.py::{import.1} -> pkg/__init__.py [import]",
            "main.py::{import.2} -> pkg/util.py [import]",
            "main.py::{import.3} -> pkg/models.py::Circle [import]",
            // A method called through `self`.
            "pkg/models.py::Shape::describe() -> pkg/models.py::Shape::area() [call]",
            "pkg/models.py::Circle::twice() -> pkg/models.py::Circle::area() [call]",
            // Fields and class attributes through `self`.
            "pkg/models.py::Circle::area() -> pkg/models.py::Circle::__init__()::radius [access]",
            "pkg/models.py::Circle::area() -> pkg/models.py::Circle::scale [access]",
            // Inheritance, and a method of the base class through `super()`.
            "pkg/models.py::Circle -> pkg/models.py::Shape [inherit]",
            "pkg/models.py::Circle::twice() -> pkg/models.py::Shape::describe() [call]",
            // An imported name under an alias, and a name through a module.
            "pkg/models.py::Circle::__init__() -> pkg/util.py::clamp() [call]",
            "pkg/models.py::Circle::twice() -> pkg/util.py::helper() [call]",
            // A class constructed, a decorator, a plain call.
            "pkg/models.py::make() -> pkg/models.py::Circle [call]",
            "pkg/util.py::clamp() -> pkg/util.py::logged() [access]",
            "pkg/util.py::helper() -> pkg/util.py::clamp() [call]",
            // From another file: a re-exported class, a dotted module path,
            // an aliased module.
            "main.py::run() -> pkg/models.py::Circle [call]",
            "main.py::run() -> pkg/models.py::make() [call]",
            "main.py::run() -> pkg/util.py::helper() [call]",
        ],
    );
    // Not found, and not expected of the fast tier: `circle.area()` in
    // `make`, a method called on a value whose type is not written down.
}
