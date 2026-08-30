"""Stage framework constants."""

_ARTIFACT_PREFIX = "artifact."

FRAMEWORK_KEYS = frozenset(
    {
        "name",
        "model",
        "artifact_dir",
        "cwd",
        "base_ref",
        "loop_iteration",
    }
)
