"""The pages of the notebooks (docs/notebooks/<n>_*.md) from their saved outputs, for the docs build.

nbconvert writes each output of a cell into one indented block, with empty lines between the outputs. A notebook shows
the outputs of a cell one under the other, so this script removes the empty lines between two output lines.

    python scripts/notebook_pages.py
"""
import re
import subprocess
import sys
from pathlib import Path

NOTEBOOKS = Path(__file__).resolve().parents[1] / "docs" / "notebooks"


def join_outputs(md: str) -> str:
    """Remove the empty lines between two lines of an output block (lines that start with 4 spaces)."""
    return re.sub(r"(?m)^(    .*)\n(?:[ \t]*\n)+(?=    \S)", r"\1\n", md)


def main():
    notebooks = sorted(NOTEBOOKS.glob("[0-9]*.ipynb"))
    subprocess.run([sys.executable, "-m", "nbconvert", "--to", "markdown", *map(str, notebooks)], check=True)
    for nb in notebooks:
        page = nb.with_suffix(".md")
        page.write_text(join_outputs(page.read_text(encoding="utf-8")), encoding="utf-8")


if __name__ == "__main__":
    assert join_outputs("    a\n\n\n\n    b\n\n```python\nx\n```\n") == "    a\n    b\n\n```python\nx\n```\n"
    main()
