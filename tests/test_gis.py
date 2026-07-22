"""Tests for duckosm.gis.to_gis — export a duckOSM network to GeoPackage / shapefile.

The point of every duckOSM exporter is that the content-hash `edge_id` survives into the output as a
plain attribute, and the geographic layers (edges/nodes/boundary) carry through while routing-only
list columns (`refs`) are dropped so the shapefile stays valid.
"""
import shutil

import duckdb
import pytest

from duckosm.gis import to_gis

E1, E2 = 6141068311830699705, 3843102655846694531
HAVE_OGR2OGR = shutil.which("ogr2ogr") is not None


def _db():
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    p = lambda w: f"ST_GeomFromText('{w}')"
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY)")
    con.execute(f"""INSERT INTO driving.nodes VALUES
        (1,{p('POINT(18.06 59.32)')}),(2,{p('POINT(18.07 59.32)')}),(3,{p('POINT(18.08 59.32)')})""")
    # `refs BIGINT[]` and `maxspeed_kmh` (>10 chars) exercise the list-drop + shp-rename paths
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, "
                "highway VARCHAR, name VARCHAR, oneway BOOLEAN, lanes INTEGER, length_m DOUBLE, "
                "maxspeed_kmh FLOAT, cost_s DOUBLE, refs BIGINT[], geometry GEOMETRY)")
    con.execute(f"""INSERT INTO driving.edges VALUES
        ({E1},1,2,'residential','A St',false,1,123.456789,30,14.8148146,[1,2],
            {p('LINESTRING(18.06 59.32,18.07 59.32)')}),
        ({E2},2,3,'tertiary',NULL,true,2,567.8,50,40.8816,[2,3],
            {p('LINESTRING(18.07 59.32,18.08 59.32)')})""")
    con.execute("CREATE TABLE boundary(osm_id INTEGER, name VARCHAR, geom GEOMETRY)")
    con.execute(f"INSERT INTO boundary VALUES (99,'area',"
                f"{p('POLYGON((18.05 59.31,18.09 59.31,18.09 59.33,18.05 59.33,18.05 59.31))')})")
    return con


def _cols(con, source, **kw):
    """Column names of a written GDAL layer, read back through DuckDB's ST_Read."""
    args = ", ".join([f"'{source}'"] + [f"{k}='{v}'" for k, v in kw.items()])
    return [c[0] for c in con.execute(f"DESCRIBE SELECT * FROM ST_Read({args})").fetchall()]


def test_shp_preserves_edge_id_and_drops_list_columns(tmp_path):
    con = _db()
    res = to_gis(con, str(tmp_path), fmt="shp", name="soder")
    assert res["fmt"] == "shp"
    assert res["layers"] == {"edges_driving": 2, "nodes_driving": 3, "boundary": 1}
    edges_shp = tmp_path / "soder_edges_driving.shp"
    for ext in ("shp", "dbf", "shx", "prj"):                 # .prj proves the CRS was written
        assert (tmp_path / f"soder_edges_driving.{ext}").exists()

    cols = _cols(con, edges_shp)
    assert "edge_id" in cols                                  # the stable id survived as an attribute
    assert "refs" not in cols                                 # BIGINT[] dropped (shp can't hold it)
    assert "spd_kmh" in cols and "maxspeed_kmh" not in cols   # >10-char field renamed for shp

    # int64 ids are cast to text for shp — a DBF numeric field would truncate the 19-digit hash
    ids = [r[0] for r in con.execute(f"SELECT edge_id FROM ST_Read('{edges_shp}')").fetchall()]
    assert set(ids) == {str(E1), str(E2)}                    # content-hash ids intact (exact, as text)


def test_length_and_cost_rounded_to_2_decimals(tmp_path):
    con = _db()
    to_gis(con, str(tmp_path), fmt="shp", name="soder")
    rows = con.execute(
        f"SELECT length_m, cost_s FROM ST_Read('{tmp_path}/soder_edges_driving.shp') "
        "ORDER BY length_m").fetchall()
    assert rows == [(123.46, 14.81), (567.8, 40.88)]


def test_only_selected_mode_exported(tmp_path):
    con = _db()
    con.execute("CREATE SCHEMA walking")
    con.execute("CREATE TABLE walking.edges(edge_id BIGINT, geometry GEOMETRY)")
    res = to_gis(con, str(tmp_path), fmt="shp", modes=["driving"])
    assert "edges_driving" in res["layers"] and "edges_walking" not in res["layers"]


def test_unknown_mode_errors(tmp_path):
    with pytest.raises(ValueError, match="not found"):
        to_gis(_db(), str(tmp_path), fmt="shp", modes=["flying"])


@pytest.mark.skipif(not HAVE_OGR2OGR, reason="ogr2ogr (GDAL) not installed")
def test_gpkg_is_single_multi_layer_file(tmp_path):
    con = _db()
    out = tmp_path / "net.gpkg"
    res = to_gis(con, str(out), fmt="gpkg")
    assert res["path"] == str(out) and out.exists()
    assert set(res["layers"]) == {"edges_driving", "nodes_driving", "boundary"}

    cols = _cols(con, out, layer="edges_driving")
    assert "edge_id" in cols and "refs" not in cols
    assert "maxspeed_kmh" in cols                             # gpkg keeps the full field name
    ids = [r[0] for r in con.execute(
        f"SELECT edge_id FROM ST_Read('{out}', layer='edges_driving')").fetchall()]
    assert set(ids) == {E1, E2}
