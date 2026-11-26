"""Tests for ``duckosm gis-debug`` — read an export back through GDAL and audit it.

Exercises the real path: export a tiny network with to_gis, then run the Rust command on the files on disk."""
from cli_run import invoke
import duckdb
import pytest

from duckosm.gis import to_gis


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


def _check(path, *extra):
    """Run `gis-debug PATH --json`: the checks as a dict."""
    import json
    r = invoke(["gis-debug", str(path), "--json", *extra])
    assert r.exit_code == 0, r.output
    return json.loads(r.output[r.output.index("{"):])


def test_payload_audits_a_clean_shapefile_export(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)                                  # the HTML page goes to the current folder
    to_gis(_db(), str(tmp_path / "shp"), fmt="shp", name="t", boundary=False)

    payload = _check(tmp_path / "shp")
    assert payload["verdict"] == "pass"
    assert payload["summary"]["crs"] == "EPSG:4326"
    assert payload["summary"]["edge_id"] == "exact"
    assert payload["summary"]["modes"] == ["driving"]

    edges = next(L for L in payload["layers"] if L["kind"] == "edges")
    assert edges["features"] == 2
    assert edges["eid_dtype"] in ("object", "str")               # shp keeps int64 ids as text
    assert any("text" in n["text"] for n in edges["notes"])      # and says so


def test_roundtrip_flags_a_dropped_edge(tmp_path, monkeypatch):
    """A round-trip diff against the source db catches an edge that didn't survive the export."""
    monkeypatch.chdir(tmp_path)
    con = _db()
    to_gis(con, str(tmp_path / "shp"), fmt="shp", name="t", boundary=False)
    # simulate a source that has an extra edge the export is missing -> round-trip should fail
    con.execute(f"""INSERT INTO driving.edges VALUES
        (777,3,1,'service',NULL,false,1,10.0,20,
         ST_GeomFromText('LINESTRING(18.08 59.32,18.06 59.32)'))""")
    db_path = str(tmp_path / "src.duckdb")
    con.execute(f"ATTACH '{db_path}' AS out"); con.execute("COPY FROM DATABASE memory TO out")
    con.execute("DETACH out"); con.close()                      # release the file before reading it back

    payload = _check(tmp_path / "shp", "--source-db", db_path)
    assert payload["verdict"] == "fail"
    assert payload["summary"]["roundtrip"] == "fail"
    edges = next(L for L in payload["layers"] if L["kind"] == "edges")
    assert edges["roundtrip"]["missing"] == 1                    # id 777 is in the db, not the export
    assert any(n["sev"] == "crit" and "round-trip" in n["text"] for n in edges["notes"])


def test_cli_writes_a_standalone_page(tmp_path):
    to_gis(_db(), str(tmp_path / "shp"), fmt="shp", name="t", boundary=False)
    page = tmp_path / "debug.html"
    r = invoke(["gis-debug", str(tmp_path / "shp"), "-o", str(page)])
    assert r.exit_code == 0, r.output
    doc = page.read_text()
    assert doc.lower().startswith("<!doctype html>") and "/*__PAYLOAD__*/" not in doc
    # geometry made it into the drawable grid, classed by highway (primary and footway differ)
    import re, json
    data = json.loads(re.search(r"const DATA = (\{.*?\});\n", doc, re.S).group(1))
    drawn = data["geo"]["driving"]["edges"]
    assert len(drawn) == 2 and len({e[0] for e in drawn}) == 2
