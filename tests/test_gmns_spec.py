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


def test_location_and_zone(tmp_path):
    """Node 4 (a crossing) lies mid-way on link A and its reverse AR; node 2 (signals) is a junction; node 5
    lies on no link, so it is not written. The area's boundary becomes one zone."""
    import duckdb
    from tests.test_gmns import A, AR, B
    _source(tmp_path / "src.duckdb")
    to_gmns(str(tmp_path / "src.duckdb"), str(tmp_path / "out.duckdb"))
    con = duckdb.connect(str(tmp_path / "out.duckdb"), read_only=True)
    q = lambda s: con.execute(s).fetchall()
    assert {r[0] for r in q("SELECT osm_id FROM gmns_driving.location")} == {2, 4}          # 5 is on no link
    rows = {(r[0], r[1]): r for r in q("SELECT osm_id, link_id, lr, loc_type, ref_node_id FROM gmns_driving.location")}
    length = {r[0]: r[1] for r in q("SELECT link_id, length FROM gmns_driving.link")}
    for link in (A, AR):                                              # the crossing is half way along each
        assert rows[(4, link)][2] == pytest.approx(length[link] / 2, rel=0.01) and rows[(4, link)][3] == "crossing"
    assert rows[(2, A)][2] == pytest.approx(length[A], rel=0.001)       # the signals: at A's end, B's and AR's start
    assert rows[(2, B)][2] == 0 and rows[(2, B)][3] == "traffic_signals" and rows[(2, B)][4] == 2
    # the zone: the boundary, once
    z = q("SELECT zone_id, name, boundary FROM gmns_driving.zone")
    assert len(z) == 1 and z[0][1] == "Testville" and z[0][2].startswith("POLYGON")


def test_footways_have_no_lanes_or_capacity_but_roads_keep_them(tmp_path):
    import duckdb
    from tests.test_gmns import A, B
    src = tmp_path / "src.duckdb"
    _source(src)
    con = duckdb.connect(str(src))
    con.execute("LOAD spatial; CREATE SCHEMA walking")
    con.execute("CREATE TABLE walking.nodes AS SELECT * FROM driving.nodes")
    con.execute("CREATE TABLE walking.edges AS SELECT * FROM driving.edges")
    con.execute(f"UPDATE walking.edges SET highway = 'footway' WHERE edge_id = {B}")   # A stays 'primary'
    con.close()
    to_gmns(str(src), str(tmp_path / "out.duckdb"), modes=["walking"])
    r = {k: (lanes, cap) for k, lanes, cap in duckdb.connect(str(tmp_path / "out.duckdb"), read_only=True).execute(
        "SELECT link_id, lanes, capacity FROM gmns_walking.link").fetchall()}
    assert r[B] == (None, None)                    # a footway: no motor lanes, uncapacitated
    assert r[A] == (2, 1600.0)                     # a road, walked on: keeps its motor lanes and capacity
