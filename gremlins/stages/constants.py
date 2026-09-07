"""Stage framework constants."""

_BAIL_KEY = "artifact://bail"

FRAMEWORK_KEYS = frozenset(
    {
        "name",
        "model",
        "cwd",
        "base_ref",
        "loop_iteration",
    }
)