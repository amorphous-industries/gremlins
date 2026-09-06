from __future__ import annotations

import argparse
import sys

from _gremlins_core.assets import load_bundled_prompt


def prompt_for_assistant_main(argv: list[str]) -> int:
    p = argparse.ArgumentParser(
        prog="gremlins prompt-for-assistant",
        description="Print the assistant setup prompt to stdout.",
        epilog="Example: gremlins prompt-for-assistant | pbcopy",
    )
    p.parse_args(argv)

    content = load_bundled_prompt("assistant/setup.md")
    sys.stdout.write(content)
    return 0
