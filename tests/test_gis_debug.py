"""Tests for duckosm.gis_debug — read an export back through GDAL and audit it.

Exercises the real path: export a tiny network with to_gis, then build the debug payload from the
files on disk. The shapefile path needs no ogr2ogr, so it runs everywhere geopandas is installed.
"""
import duckdb
import pytest

from duckosm.gis import to_gis

pytest.importorskip("geopandas")
from duckosm.gis_debug import build_payload, render_html, _class_index  # noqa: E402

E1, E2 = 6141068311830699705, 3843102655846694531


def _db():
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    p = lambda w: f"ST_GeomFromText('{w}')"
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY)")
    con.execute(f"""INSERT INTO driving.nodes VALUES
        (1,{p('POINT(18.06 59.32)')}),(2,{p('POINT(18.07 59.32)')}),(3,{p('POINT(18.08 59.32)')})""")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, "
                "highway VARCHAR, name VARCHAR, oneway BOOLEAN, lanes INTEGER, length_m FLOAT, "
                "maxspeed_kmh FLOAT, geometry GEOMETRY)")
    con.execute(f"""INSERT INTO driving.edges VALUES
        ({E1},1,2,'primary','Big St',false,2,123.4,50,{p('LINESTRING(18.06 59.32,18.07 59.32)')}),
        ({E2},2,3,'footway',NULL,true,1,567.8,5,{p('LINESTRING(18.07 59.32,18.08 59.32)')})""")
    return con


def test_payload_audits_a_clean_shapefile_export(tmp_path):
    con = _db()
    to_gis(con, str(tmp_path), fmt="shp", name="t", boundary=False)

    payload = build_payload(str(tmp_path))
    assert payload["verdict"] == "pass"
    assert payload["summary"]["crs"] == "EPSG:4326"
    assert payload["summary"]["edge_id"] == "exact"
    assert payload["summary"]["modes"] == ["driving"]

    edges = next(L for L in payload["layers"] if L["kind"] == "edges")
    assert edges["features"] == 2
    assert edges["eid_dtype"] in ("object", "str")               # shp keeps int64 ids as text (pandas 3 names that dtype "str")
    assert any("text" in n["text"] for n in edges["notes"])      # and says so

    # geometry made it into the drawable grid, classed by highway (primary vs footway differ)
    drawn = payload["geo"]["driving"]["edges"]
    assert len(drawn) == 2
    assert {e[0] for e in drawn} == {_class_index("primary"), _class_index("footway")}


def test_roundtrip_flags_a_dropped_edge(tmp_path):
    """A round-trip diff against the source db catches an edge that didn't survive the export."""
    con = _db()
    to_gis(con, str(tmp_path), fmt="shp", name="t", boundary=False)
    # simulate a source that has an extra edge the export is missing -> round-trip should fail
    con.execute(f"""INSERT INTO driving.edges VALUES
        (777,3,1,'service',NULL,false,1,10.0,20,
         ST_GeomFromText('LINESTRING(18.08 59.32,18.06 59.32)'))""")
    db_path = str(tmp_path / "src.duckdb")
    con.execute(f"ATTACH '{db_path}' AS out"); con.execute("COPY FROM DATABASE memory TO out")
    con.execute("DETACH out"); con.close()                      # release the file before reading it back

    payload = build_payload(str(tmp_path), source_db=db_path)
    assert payload["verdict"] == "fail"
    assert payload["summary"]["roundtrip"] == "fail"
    edges = next(L for L in payload["layers"] if L["kind"] == "edges")
    assert edges["roundtrip"]["missing"] == 1                    # id 777 is in the db, not the export
    assert any(n["sev"] == "crit" and "round-trip" in n["text"] for n in edges["notes"])


def test_render_html_standalone_and_body(tmp_path):
    con = _db()
    to_gis(con, str(tmp_path), fmt="shp", name="t", boundary=False)
    payload = build_payload(str(tmp_path))

    doc = render_html(payload, standalone=True)
    assert doc.startswith("<!doctype html>") and "const DATA = {" in doc and "/*__PAYLOAD__*/" not in doc
    body = render_html(payload, standalone=False)
    assert "<!doctype" not in body and body.startswith("<title>")


def test_cli_json(tmp_path, monkeypatch):
    """`gis-debug --json` prints the check for an agent: the verdict, the summary, each layer."""
    import json
    from click.testing import CliRunner
    from duckosm.cli import main
    to_gis(_db(), str(tmp_path / "shp"), fmt="shp", name="t", boundary=False)
    monkeypatch.chdir(tmp_path)
    r = CliRunner().invoke(main, ["gis-debug", str(tmp_path / "shp"), "--json"])
    assert r.exit_code == 0, r.output
    d = json.loads(r.output[r.output.index("{"):])
    assert d["verdict"] == "pass" and d["summary"]["modes"] == ["driving"] and d["layers"]
