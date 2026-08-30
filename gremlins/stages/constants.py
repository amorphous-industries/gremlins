"""Stage framework constants."""

ARTIFACT_PREFIX = "artifact."

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


def strip_artifact_prefix(raw: dict[str, str], name: str) -> dict[str, str]:
    result: dict[str, str] = {}
    for k, v in raw.items():
        if v.startswith(ARTIFACT_PREFIX):
            result[k] = v[len(ARTIFACT_PREFIX) :]
        else:
            raise ValueError(
                f"stage {name!r}: interpolation value {v!r} must start with 'artifact.'"
            )
    return result


def strip_artifact_prefix_keys(raw: dict[str, str], name: str) -> dict[str, str]:
    result: dict[str, str] = {}
    for k, v in raw.items():
        if k.startswith(ARTIFACT_PREFIX):
            result[k[len(ARTIFACT_PREFIX) :]] = v
        else:
            raise ValueError(
                f"stage {name!r}: bind key {k!r} must start with 'artifact.'"
            )
    return result
