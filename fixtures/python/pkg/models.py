"""Classes, attributes and the ways Python defines them."""
# construct: module-docstring

# construct: future-import
from __future__ import annotations

# construct: import
import math

# construct: from-import
from dataclasses import dataclass, field
from enum import Enum
from typing import TYPE_CHECKING, Protocol, overload

# construct: type-checking-block
if TYPE_CHECKING:
    from collections.abc import Iterable

# construct: try-import
try:
    import ujson as json
except ImportError:
    import json

# construct: module-constant
TWO_PI = math.pi * 2

# construct: module-variable-annotated
default_unit: str = "m"

# construct: private-underscore
_registry: dict[str, Shape] = {}

# construct: type-alias-statement
type Number = int | float


# construct: protocol
class Shape(Protocol):
    def area(self) -> float: ...


# construct: enum-class
class Unit(Enum):
    # construct: enum-member
    METER = "m"
    FOOT = "ft"


# construct: class
class Circle:
    """A circle with a cached area."""

    # construct: class-attribute
    sides = 0

    # construct: class-attribute-annotated
    kind: str = "circle"

    # construct: constructor
    def __init__(self, radius: Number, unit: Unit = Unit.METER) -> None:
        # construct: instance-attribute
        self.radius = radius
        self.unit = unit
        # construct: private-dunder
        self.__cache: float | None = None

    # construct: method
    def area(self) -> float:
        if self.__cache is None:
            self.__cache = math.pi * self.radius**2
        return self.__cache

    # construct: property
    @property
    def diameter(self) -> Number:
        return self.radius * 2

    # construct: property-setter
    @diameter.setter
    def diameter(self, value: Number) -> None:
        self.radius = value / 2
        self._invalidate()

    # construct: staticmethod
    @staticmethod
    def unit_circle() -> Circle:
        return Circle(1)

    # construct: classmethod
    @classmethod
    def from_diameter(cls, diameter: Number) -> Circle:
        return cls(diameter / 2)

    # construct: dunder-method
    def __repr__(self) -> str:
        return f"Circle({self.radius!r})"

    # construct: private-method
    def _invalidate(self) -> None:
        self.__cache = None

    # construct: nested-class
    class Meta:
        ordering = ("radius",)


# construct: inheritance
class Ring(Circle):
    def __init__(self, radius: Number, inner: Number) -> None:
        super().__init__(radius)
        self.inner = inner

    # construct: method-override
    def area(self) -> float:
        return super().area() - math.pi * self.inner**2


# construct: dataclass
@dataclass(frozen=True)
class Point:
    # construct: dataclass-field
    x: float
    y: float = 0.0
    tags: list[str] = field(default_factory=list)


# construct: overload
@overload
def scale(shape: Ring, factor: float) -> Ring: ...
@overload
def scale(shape: Circle, factor: float) -> Circle: ...
def scale(shape, factor):
    if isinstance(shape, Ring):
        return Ring(shape.radius * factor, shape.inner * factor)
    return Circle(shape.radius * factor)


def total(shapes: Iterable[Shape]) -> str:
    return json.dumps({"area": sum(shape.area() for shape in shapes)})
