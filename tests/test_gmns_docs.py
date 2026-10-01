"""docs/exports/gmns_tables.md says what duckOSM writes: this fails when it stops being true.

For every table the page documents, its column tables must list exactly the columns of the real table, mark
the standard ones (✅ / ∅) as the vendored spec schema has them and the extensions (➕) as it does not, and
the CSV export must carry exactly the standard ones.
"""
import csv
import json
import re
from pathlib import Path

import duckdb
import pytest

pytest.importorskip("pandas")
from duckosm.gmns import to_gmns  # noqa: E402
from tests.test_gmns import _source  # noqa: E402

DOC = Path(__file__).parents[1] / "docs" / "exports" / "gmns_tables.md"
SPEC = Path(__file__).parent / "data" / "gmns_spec"


def _documented():
    """{table: {column: mark}} from the page: each ``## <table>`` section's first table, whose rows start with
    the backticked column names (several to a row) and, when there is a Spec column, its mark."""
    out, table = {}, None
    for line in DOC.read_text().splitlines():
        m = re.match(r"## (\w+)\s*$", line)
        if m:
            table = m.group(1)
            continue
        if line.startswith("## "):
            table = None
        if table is None or not line.startswith("|") or line.startswith("|---") or line.startswith("| Column"):
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        mark = cells[2] if len(cells) == 4 else "➕"                # lane_connector: no Spec column
        for col in re.findall(r"`(\w+)`", cells[0]):
            out.setdefault(table, {})[col] = mark.split()[0]
    return out


@pytest.fixture(scope="module")
def built(tmp_path_factory):
    d = tmp_path_factory.mktemp("docs")
    _source(d / "src.duckdb")
    to_gmns(str(d / "src.duckdb"), str(d / "out.duckdb"), to_csv=str(d / "csv"))
    return d


def test_the_page_documents_every_table_the_output_has(built):
    con = duckdb.connect(str(built / "out.duckdb"), read_only=True)
    real = {t for (t,) in con.execute("SELECT table_name FROM information_schema.tables "
                                      "WHERE table_schema = 'gmns_driving'").fetchall()}
    assert real == set(_documented()) - {"gmns_all"} | set(), (real ^ set(_documented()))


def test_each_documented_table_has_the_real_columns_and_marks(built):
    con = duckdb.connect(str(built / "out.duckdb"), read_only=True)
    doc = _documented()
    for table, cols in doc.items():
        real = [c for (c,) in con.execute("SELECT column_name FROM information_schema.columns WHERE "
                                          "table_schema = 'gmns_driving' AND table_name = ?", [table]).fetchall()]
        if not real:
            continue                                               # gmns_all, meso, micro: not in this build
        assert set(cols) == set(real), (table, set(cols) ^ set(real))
        schema = SPEC / f"{table}.schema.json"
        if not schema.exists():                                    # lane_connector: ours
            assert set(cols.values()) == {"➕"}
            continue
        spec = {f["name"] for f in json.loads(schema.read_text())["fields"]}
        standard = {c for c, m in cols.items() if m in ("✅", "∅")}
        assert standard == spec, (table, standard ^ spec)           # ✅ / ∅ are exactly the standard's columns
        with open(built / "csv" / f"{table}.csv", newline="") as fh:
            assert set(next(csv.reader(fh))) == spec, table        # and the CSV has just those
