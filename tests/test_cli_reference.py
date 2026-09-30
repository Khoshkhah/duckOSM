"""The Command line reference page (docs/reference/cli.md) documents every `duckosm` command."""
from pathlib import Path

from duckosm.cli import main

PAGE = Path(__file__).resolve().parents[1] / "docs" / "reference" / "cli.md"


def test_every_command_is_in_the_cli_reference():
    text = PAGE.read_text(encoding="utf-8")
    missing = sorted(c for c in main.commands if f"### `{c}`" not in text)
    assert not missing, f"commands missing from docs/reference/cli.md: {missing}"
