"""Import-time side effect: ensures _gremlins_core.clients is imported.

The four built-in providers (openai, xai, openrouter, cmd) are handled
natively in Rust and do not need factory registrations. CLIENT_FACTORIES
is populated dynamically by tests and user-specified custom providers.
"""

from __future__ import annotations

from _gremlins_core.clients import (  # noqa: F401 — import side effect
    CLIENT_FACTORIES,
)