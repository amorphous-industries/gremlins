"""Central gremlins configuration from ~/.config/gremlins/config.json.

Usage (production):
    from gremlins.config import get_config
    cfg = get_config()
    client = cfg.default_client         # "default-client" value or None
    by_stage = cfg.default_client_by_stage  # prefix → client spec

The CLI entry point calls ``init()`` at startup so the singleton is
ready before any stage code runs.  Lazy callers via ``get_config()``
auto-load on first access.

Tests: each test gets a unique sandbox and ``Config`` is stateless for
the duration of one test.  The autouse ``_reset_config`` fixture in
``tests/conftest.py`` clears the singleton between tests so the next
``get_config()`` picks up the new sandbox's ``config.json``.
"""

from __future__ import annotations

import json
import logging
from typing import Any, cast

from gremlins.paths import user_config_root

logger = logging.getLogger(__name__)


class Config:
    """Settings loaded from ~/.config/gremlins/config.json."""

    __slots__ = ("_data",)

    def __init__(self) -> None:
        self._data: dict[str, Any] = {}

    def load(self) -> None:
        """Read config.json from the user config directory."""
        path = user_config_root() / "config.json"
        try:
            with open(path, encoding="utf-8") as f:
                data = json.load(f)
        except FileNotFoundError:
            self._data = {}
            return
        except json.JSONDecodeError as e:
            raise ValueError(f"config file {path} is not valid JSON: {e}") from e
        if not isinstance(data, dict):
            raise ValueError(
                f"config file {path} must contain a JSON object, "
                f"got {type(data).__name__}"
            )
        self._data = cast(dict[str, Any], data)

    @property
    def default_client(self) -> str | None:
        """Return the global ``default-client`` setting, or None."""
        val = self._data.get("default-client")
        return val if isinstance(val, str) and val else None

    @property
    def default_client_by_stage(self) -> dict[str, str]:
        """Return prefix → client-spec from ``default-client-by-stage``.

        Keys ending with ``*`` are prefix globs (the ``*`` is stripped).
        Keys without ``*`` are exact name matches.  Returns a flat dict
        where exact-name keys are stored as-is and prefix keys have their
        trailing ``*`` removed.

        When a stage name matches both an exact key and a prefix key, the
        exact match takes priority (enforced by ``_bake_prefix_clients``
        which checks the exact map first).

        Invalid entries are skipped with a warning.
        """
        raw = self._data.get("default-client-by-stage")
        if not isinstance(raw, dict):
            return {}
        result: dict[str, str] = {}
        for key, value in cast(dict[str, Any], raw).items():
            if not isinstance(value, str):
                logger.warning(
                    "config key %r in default-client-by-stage has "
                    "non-string value %r — skipping",
                    key,
                    value,
                )
                continue
            if key.endswith("*"):
                prefix = key[:-1]
                if not prefix:
                    logger.warning(
                        "config key %r in default-client-by-stage produces "
                        "an empty prefix, which would match every stage — "
                        "skipping",
                        key,
                    )
                    continue
                result[prefix] = value
            else:
                # Exact name match — store the key as-is.
                result[key] = value
        return result

    @property
    def raw(self) -> dict[str, Any]:
        """Return the raw parsed dict for callers that need direct access."""
        return self._data


# Module-level singleton.  The CLI calls init() at startup; lazy
# callers via get_config() auto-create on first access.  Tests call
# clear() between sandboxed cases (or rely on the conftest autouse
# fixture).
_config: Config | None = None


def init() -> None:
    """Load config from disk into the module-level singleton."""
    global _config
    _config = Config()
    _config.load()


def get_config() -> Config:
    """Return the module-level config singleton (lazy-loads if unset)."""
    global _config
    if _config is None:
        _config = Config()
        _config.load()
    return _config


def clear() -> None:
    """Reset the singleton so the next get_config() re-reads from disk.

    Intended for tests where the sandbox changes between cases.
    """
    global _config
    _config = None
