"""Write the two catalog tables of docs/reference/elevation-sources.md from src/duckosm/data/dem_sources.json.

    python scripts/update_elevation_sources_doc.py          # rewrite the tables
    python scripts/update_elevation_sources_doc.py --check  # stop with exit code 1 when the page differs (tests/test_dem_sources.py does this)
"""
import re
import sys
from pathlib import Path

from duckosm.dem_sources import render_tables

DOC = Path(__file__).resolve().parent.parent / "docs/reference/elevation-sources.md"


def updated(text: str) -> str:
    for name, table in render_tables().items():
        pat = re.compile(rf"(<!-- dem-sources:{name} -->\n).*?(\n<!-- /dem-sources:{name} -->)", re.S)
        if not pat.search(text):
            raise SystemExit(f"{DOC}: the markers dem-sources:{name} are missing")
        text = pat.sub(lambda m: m.group(1) + table + m.group(2), text)
    return text


if __name__ == "__main__":
    old = DOC.read_text(encoding="utf-8")
    new = updated(old)
    if "--check" in sys.argv:
        sys.exit(0 if new == old else 1)
    DOC.write_text(new, encoding="utf-8")
