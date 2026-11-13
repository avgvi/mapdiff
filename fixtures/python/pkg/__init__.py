"""Hand-made fixture package.

Every Python construct the extractor must map appears in this package, tagged
with a ``construct:`` marker on the line above it. The list of required markers
is ``fixtures/constructs/python.txt``.
"""
# construct: package-init

# construct: relative-import
from .models import Circle, Shape

# construct: import-alias
from . import service as svc

# construct: dunder-all
__all__ = ["Circle", "Shape", "svc"]
