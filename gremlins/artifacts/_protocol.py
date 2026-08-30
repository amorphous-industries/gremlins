"""SchemeResolver protocol."""

from __future__ import annotations

from typing import Any, Protocol, runtime_checkable


@runtime_checkable
class SchemeResolver(Protocol):
    def materialize(self, uri_str: str) -> Any: ...
    def read(self, value: Any) -> Any: ...
    def verify_produced(self, value: Any) -> None: ...
