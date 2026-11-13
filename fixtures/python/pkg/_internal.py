"""A module that is private by convention (leading underscore)."""
# construct: private-module


# construct: private-function
def _clamp(value, low, high):
    return max(low, min(high, value))


# construct: module-level-statement
_clamp.calls = 0
