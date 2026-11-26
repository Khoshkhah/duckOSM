"""Tests for `duckosm elevation` (duckosm.to_elevation) — DEM sampling into a built db (nodes.ele + edges.z_from/z_to)."""
import duckdb
import pytest

pytest.importorskip("rasterio")
pytest.importorskip("pyproj")

from duckosm import to_elevation

A, B = 6141068311830699705, 3843102655846694531


def _ramp(lon, lat):
    """Known east-west ramp: elevation rises 1000 m per degree of longitude east of 18°."""
    return (lon - 18.0) * 1000.0


def _dem(path, west=17.9, south=59.2, east=18.2, north=59.4, n=600):
    """Write a synthetic EPSG:4326 GeoTIFF of the ramp over the test bbox."""
    import numpy as np
    import rasterio
    from rasterio.transform import from_bounds
    data = np.empty((n, n), "float32")
    for r in range(n):
        lat = north - (r + 0.5) / n * (north - south)
        for c in range(n):
            lon = west + (c + 0.5) / n * (east - west)
            data[r, c] = _ramp(lon, lat)
    with rasterio.open(path, "w", driver="GTiff", height=n, width=n, count=1, dtype="float32",
                       crs="EPSG:4326", transform=from_bounds(west, south, east, north, n, n),
                       nodata=-9999.0) as dst:
        dst.write(data, 1)
    return str(path)


def _src(path, nodes, edges):
    """Minimal driving-only db: nodes=(id,lon,lat), edges=(edge_id,source,target)."""
    con = duckdb.connect(str(path))
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY)")
    for nid, lon, lat in nodes:
        con.execute(f"INSERT INTO driving.nodes VALUES ({nid}, ST_GeomFromText('POINT({lon} {lat})'))")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, "
                "highway VARCHAR, geometry GEOMETRY)")
    g = "ST_GeomFromText('LINESTRING(18.06 59.32,18.07 59.32)')"
    for eid, src, tgt in edges:
        con.execute(f"INSERT INTO driving.edges VALUES ({eid},{src},{tgt},'primary',{g})")
    con.close()
    return str(path)


def test_ele_and_z_from_local_dem(tmp_path):
    dem = _dem(tmp_path / "ramp.tif")
    nodes = [(1, 18.06, 59.32), (2, 18.07, 59.32)]
    src = _src(tmp_path / "s.duckdb", nodes, [(A, 1, 2)])
    res = to_elevation(src, dem=dem, reference=False)

    assert res["n_nodes"] == 2 and res["n_nodata"] == 0
    con = duckdb.connect(src, read_only=True)
    ele = dict(con.execute("SELECT node_id, ele FROM driving.nodes").fetchall())
    assert ele[1] == pytest.approx(_ramp(18.06, 59.32), abs=2.0)   # ~60 m
    assert ele[2] == pytest.approx(_ramp(18.07, 59.32), abs=2.0)   # ~70 m
    assert all(v == round(v, 2) for v in ele.values())             # stored to 2 dp (cm)
    # edge endpoints carry the node elevations
    z = con.execute("SELECT z_from, z_to FROM driving.edges WHERE edge_id = ?", [A]).fetchone()
    assert z[0] == pytest.approx(ele[1], abs=1e-4) and z[1] == pytest.approx(ele[2], abs=1e-4)
    con.close()


def test_nodata_fill_out_of_coverage(tmp_path):
    dem = _dem(tmp_path / "ramp.tif")
    # node 3 sits far outside the DEM bbox → must get the fill value, and count as nodata
    src = _src(tmp_path / "s.duckdb", [(1, 18.06, 59.32), (2, 18.07, 59.32), (3, 100.0, 10.0)],
               [(A, 1, 2), (B, 2, 3)])
    res = to_elevation(src, dem=dem, nodata_fill=-999.0, reference=False)

    assert res["n_nodata"] == 1
    con = duckdb.connect(src, read_only=True)
    assert con.execute("SELECT ele FROM driving.nodes WHERE node_id = 3").fetchone()[0] == -999.0
    # edge B (…→3) has the fill on its z_to
    assert con.execute("SELECT z_to FROM driving.edges WHERE edge_id = ?", [B]).fetchone()[0] == -999.0
    con.close()


def test_provenance_metadata(tmp_path):
    dem = _dem(tmp_path / "ramp.tif")
    src = _src(tmp_path / "s.duckdb", [(1, 18.06, 59.32), (2, 18.07, 59.32)], [(A, 1, 2)])
    to_elevation(src, dem=dem, product="DTM", vertical_datum="EGM2008", license="CC-BY-4.0")

    con = duckdb.connect(src, read_only=True)
    row = con.execute("SELECT source_type, product, n_nodes, n_nodata, dem_crs, vertical_datum, "
                      "license, sha256 FROM main.elevation_metadata").fetchone()
    assert row[0] == "file" and row[1] == "DTM"           # a --dem file: type file, the given product
    assert row[2] == 2 and row[3] == 0
    assert "4326" in row[4]                                # CRS read from the raster
    assert row[5:7] == ("EGM2008", "CC-BY-4.0")
    import hashlib
    with open(dem, "rb") as f:
        assert row[7] == hashlib.sha256(f.read()).hexdigest()   # the ground id
    assert con.execute("SELECT count(*) FROM main.elevation_metadata").fetchone()[0] == 1
    con.close()


def test_reference_dem_needs_datum_and_license(tmp_path):
    dem = _dem(tmp_path / "ramp.tif")
    src = _src(tmp_path / "s.duckdb", [(1, 18.06, 59.32)], [])
    with pytest.raises(Exception, match="reference ground"):
        to_elevation(src, dem=dem, vertical_datum="EGM2008", license="x")
    with pytest.raises(Exception, match="DTM|DSM"):
        to_elevation(src, dem=dem, product="DEM", reference=False)
    to_elevation(src, dem=dem, reference=False)            # not the reference: unknown is allowed
    con = duckdb.connect(src, read_only=True)
    assert con.execute("SELECT vertical_datum, license FROM main.elevation_metadata").fetchone() == (
        "unknown", "unknown")
    con.close()


def test_unknown_source_errors(tmp_path):
    src = _src(tmp_path / "s.duckdb", [(1, 18.06, 59.32)], [])
    with pytest.raises(Exception, match="unknown --source"):
        to_elevation(src, source="nope")


def test_suffix_keeps_two_surfaces(tmp_path):
    """A DTM in `ele` and a DSM in `ele_dsm` coexist — that difference is the whole point
    (object height above ground), so the second pass must not clobber the first."""
    ground = _dem(tmp_path / "ground.tif")
    # a "surface" 12 m above the same ramp everywhere — stands in for trees/buildings
    surface = _dem(tmp_path / "surface.tif")
    import rasterio
    with rasterio.open(surface, "r+") as ds:
        ds.write(ds.read(1) + 12.0, 1)

    nodes = [(1, 18.06, 59.32), (2, 18.07, 59.32)]
    src = _src(tmp_path / "s.duckdb", nodes, [(A, 1, 2)])
    to_elevation(src, dem=ground, product="DTM", vertical_datum="EGM2008", license="test")
    res = to_elevation(src, dem=surface, suffix="dsm")
    assert res["column"] == "ele_dsm"

    con = duckdb.connect(src, read_only=True)
    row = con.execute("SELECT ele, ele_dsm, ele_dsm - ele FROM driving.nodes "
                      "WHERE node_id = 1").fetchone()
    assert row[0] == pytest.approx(_ramp(18.06, 59.32), abs=2.0)   # DTM survived the 2nd pass
    assert row[2] == pytest.approx(12.0, abs=0.5)                  # object height
    z = con.execute("SELECT z_from, z_from_dsm FROM driving.edges WHERE edge_id = ?",
                    [A]).fetchone()
    assert z[1] - z[0] == pytest.approx(12.0, abs=0.5)             # edges carry both too

    # one provenance row per column, and the DTM's row is still there
    meta = dict(con.execute("SELECT ele_column, n_nodes FROM main.elevation_metadata").fetchall())
    assert set(meta) == {"ele", "ele_dsm"}
    con.close()


def test_suffix_rejects_non_identifier(tmp_path):
    # the suffix is interpolated into DDL — anything but a bare identifier must be refused
    src = _src(tmp_path / "s.duckdb", [(1, 18.06, 59.32)], [])
    with pytest.raises(Exception, match="bare identifier"):
        to_elevation(src, dem=_dem(tmp_path / "r.tif"), suffix="dsm; DROP TABLE driving.nodes")


def test_z_vertices_interior(tmp_path):
    """Every vertex of an edge gets a height, in the order of the geometry; the ends equal z_from and z_to."""
    dem = _dem(tmp_path / "ramp.tif")
    src = _src(tmp_path / "s.duckdb", [(1, 18.06, 59.32), (2, 18.08, 59.32)], [(A, 1, 2)])
    con = duckdb.connect(src)
    con.execute("LOAD spatial; UPDATE driving.edges SET geometry = "
                "ST_GeomFromText('LINESTRING(18.06 59.32,18.07 59.33,18.08 59.32)')")
    con.close()
    to_elevation(src, dem=dem, reference=False)
    con = duckdb.connect(src, read_only=True)
    zf, zt, zv = con.execute("SELECT z_from, z_to, z_vertices FROM driving.edges").fetchone()
    assert len(zv) == 3 and zv[0] == zf and zv[-1] == zt
    assert zv[1] == pytest.approx(_ramp(18.07, 59.33), abs=2.0)
    con.close()
