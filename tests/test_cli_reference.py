"""The Command line reference page (docs/reference/cli.md) documents every `duckosm` command."""
from pathlib import Path

from cli_run import invoke

PAGE = Path(__file__).resolve().parents[1] / "docs" / "reference" / "cli.md"


def test_every_command_is_in_the_cli_reference():
    text = PAGE.read_text(encoding="utf-8")
    commands = invoke(["commands"]).stdout.split()
    assert "build" in commands and "sumo" in commands
    headings = " ".join(line for line in text.splitlines() if line.startswith("### "))     # a heading can name some commands
    missing = sorted(c for c in commands if f"`{c}`" not in headings)
    assert not missing, f"commands missing from docs/reference/cli.md: {missing}"
