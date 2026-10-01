"""duckOSM's GMNS CSV output against the GMNS standard (docs/design/gmns_spec_conformance.md).

Validates ``to_gmns(..., to_csv=...)`` with frictionless against the spec's own table schemas
(vendored in tests/data/gmns_spec, release v0.97), then checks the values the spec lists as
categories, which frictionless does not enforce. Skipped without ``frictionless``.
"""
import csv
import json
import shutil
from pathlib import Path

import pytest

pytest.importorskip("pandas")
frictionless = pytest.importorskip("frictionless")
from duckosm.gmns import to_gmns  # noqa: E402
from tests.test_gmns import _source  # noqa: E402

SPEC = Path(__file__).parent / "data" / "gmns_spec"


def _categories(field, shared):
    cats = field["categories"]
    if isinstance(cats, dict):                                   # {"$ref": "shared_categories.json#/x/categories"}
        cats = shared[cats["$ref"].split("#/")[1].split("/")[0]]["categories"]
    return {c if isinstance(c, str) else str(c.get("value", c.get("name"))) for c in cats}


def test_csv_output_conforms_to_the_gmns_spec(tmp_path):
    _source(tmp_path / "src.duckdb")
    out = tmp_path / "csv"
    to_gmns(str(tmp_path / "src.duckdb"), str(tmp_path / "out.duckdb"), to_csv=str(out))
    written = {p.stem for p in out.glob("*.csv")}
    assert {"node", "link", "lane", "movement", "config", "use_definition"} <= written

    # a datapackage of just the tables we write (the spec's has 27; foreign keys to the rest dropped)
    pkg = json.loads((SPEC / "datapackage.json").read_text())
    pkg["resources"] = [r for r in pkg["resources"] if r["name"] in written]
    shutil.copy(SPEC / "shared_categories.json", out)
    for r in pkg["resources"]:
        sch = json.loads((SPEC / r["schema"]).read_text())
        sch["foreignKeys"] = [f for f in sch.get("foreignKeys", []) if f["reference"]["resource"] in written | {""}]
        (out / r["schema"]).write_text(json.dumps(sch))
    (out / "datapackage.json").write_text(json.dumps(pkg))

    report = frictionless.validate(str(out / "datapackage.json"))
    errors = [(t.name, e.message) for t in report.tasks for e in t.errors]
    assert not errors, errors[:5]

    shared = json.loads((SPEC / "shared_categories.json").read_text())
    for r in pkg["resources"]:
        sch = json.loads((out / r["schema"]).read_text())
        for f in (f for f in sch["fields"] if "categories" in f):
            allowed = _categories(f, shared)
            with open(out / r["path"], newline="") as fh:
                bad = {row[f["name"]] for row in csv.DictReader(fh) if row[f["name"]]} - allowed
            assert not bad, (r["name"], f["name"], bad)


def test_osm_facilities_and_uturns_use_the_specs_values(tmp_path):
    import duckdb
    _source(tmp_path / "src.duckdb")
    to_gmns(str(tmp_path / "src.duckdb"), str(tmp_path / "out.duckdb"))
    con = duckdb.connect(str(tmp_path / "out.duckdb"), read_only=True)
    q = lambda s: con.execute(s).fetchall()
    # way 101 is sidewalk=separate, cycleway=track; way 100 has neither tag
    assert q("SELECT DISTINCT bike_facility, ped_facility FROM gmns_driving.link WHERE bike_facility IS NOT NULL") \
        == [("separated bike lane", "offstreet_path")]
    assert q("SELECT count(*) FROM gmns_driving.link WHERE bike_facility IS NULL")[0][0] == 2
    # the spec's movement code has no U: a U-turn keeps `type`, with no code
    assert q("SELECT count(*), count(mvmt_code) FROM gmns_driving.movement WHERE type = 'uturn'") == [(1, 0)]
