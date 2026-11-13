"""Functions: plain, async, decorated, nested and redefined."""

import functools
from collections.abc import Iterable, Iterator

from ._internal import _clamp
from .models import Circle, Shape


# construct: function
def total_area(shapes: Iterable[Shape]) -> float:
    return sum(shape.area() for shape in shapes)


# construct: decorator-definition
def logged(func):
    # construct: nested-function
    @functools.wraps(func)
    def wrapper(*args, **kwargs):
        print(f"calling {func.__name__}")
        return func(*args, **kwargs)

    return wrapper


# construct: decorated-function
@logged
def biggest(shapes: Iterable[Shape]) -> Shape:
    # construct: lambda
    return max(shapes, key=lambda shape: shape.area())


# construct: async-function
async def fetch_shapes(count: int) -> list[Circle]:
    return [Circle(_clamp(index, 1, 10)) for index in range(count)]


# construct: generator-function
def radii(shapes: Iterable[Circle]) -> Iterator[float]:
    for shape in shapes:
        yield shape.radius


class ShapeService:
    def __init__(self) -> None:
        self.shapes: list[Circle] = []

    # construct: async-method
    async def refresh(self) -> None:
        self.shapes = await fetch_shapes(3)


# construct: duplicate-definition
def describe(shape: Shape) -> str:
    return "shape"


def describe(shape: Shape) -> str:  # noqa: F811
    return f"shape with area {shape.area():.2f}"


# construct: conditional-definition
if hasattr(functools, "cache"):

    @functools.cache
    def unit_area() -> float:
        return Circle(1).area()

else:

    def unit_area() -> float:
        return Circle(1).area()


# construct: main-guard
if __name__ == "__main__":
    print(total_area([Circle(1)]))
