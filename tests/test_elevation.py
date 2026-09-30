"""Tests for duckosm.elevation — DEM sampling into a built db (nodes.ele + edges.z_from/z_to)."""
import duckdb
import pytest

pytest.importorskip("rasterio")
pytest.importorskip("pyproj")

from duckosm.elevation import (to_elevation, _copernicus_url, _fix_polluted_proj,
                               _resolve_provider, _covers_europe)

A, B = 6141068311830699705, 3843102655846694531


def _ramp(lon, lat):
    """Known east-west ramp: elevation rises 1000 m per degree of longitude east of 18°."""
    return (lon - 18.0) * 1000.0


def _dem(path, west=17.9, south=59.2, east=18.2, north=59.4, n=600):
    """Write a synthetic EPSG:4326 GeoTIFF of the ramp over the test bbox."""
    _fix_polluted_proj()          # a co-run sumo test may have poisoned PROJ_DATA before this
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


def test_copernicus_tile_naming():
    # Stockholm (~59.3N, 18.1E) → N59/E018; SW-corner naming; hemispheres.
    assert "N59_00_E018_00" in _copernicus_url(59, 18)
    assert "S34_00_W071_00" in _copernicus_url(-34, -71)   # Santiago-ish
    assert _copernicus_url(59, 18).startswith("/vsicurl/https://")


def test_ele_and_z_from_local_dem(tmp_path):
    dem = _dem(tmp_path / "ramp.tif")
    nodes = [(1, 18.06, 59.32), (2, 18.07, 59.32)]
    src = _src(tmp_path / "s.duckdb", nodes, [(A, 1, 2)])
    res = to_elevation(src, dem=dem)

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
    res = to_elevation(src, dem=dem, nodata_fill=-999.0)

    assert res["n_nodata"] == 1
    con = duckdb.connect(src, read_only=True)
    assert con.execute("SELECT ele FROM driving.nodes WHERE node_id = 3").fetchone()[0] == -999.0
    # edge B (…→3) has the fill on its z_to
    assert con.execute("SELECT z_to FROM driving.edges WHERE edge_id = ?", [B]).fetchone()[0] == -999.0
    con.close()


def test_provenance_metadata(tmp_path):
    dem = _dem(tmp_path / "ramp.tif")
    src = _src(tmp_path / "s.duckdb", [(1, 18.06, 59.32), (2, 18.07, 59.32)], [(A, 1, 2)])
    to_elevation(src, dem=dem)

    con = duckdb.connect(src, read_only=True)
    row = con.execute("SELECT source_type, product, n_nodes, n_nodata, dem_crs "
                      "FROM main.elevation_metadata").fetchone()
    assert row[0] == "file" and row[1] == "unknown"       # a --dem file: type file, product unknown
    assert row[2] == 2 and row[3] == 0
    assert "4326" in row[4]                                # CRS read from the raster
    assert con.execute("SELECT count(*) FROM main.elevation_metadata").fetchone()[0] == 1
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
    to_elevation(src, dem=ground)
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
    with pytest.raises(ValueError, match="bare identifier"):
        to_elevation(src, dem=_dem(tmp_path / "r.tif"), suffix="dsm; DROP TABLE driving.nodes")


# ---- auto source resolution (pure logic, no network / no db) --------------------------------
SWEDEN = (18.0, 59.3, 18.1, 59.35)          # inside Europe
VANCOUVER = (-123.2, 49.2, -123.0, 49.3)    # outside Europe


def test_covers_europe():
    assert _covers_europe(SWEDEN) and _covers_europe((26.6, 58.3, 26.8, 58.4))   # Tartu
    assert not _covers_europe(VANCOUVER)
    assert not _covers_europe((-30.0, 40.0, -26.0, 45.0))   # mid-Atlantic, west of coverage edge


def test_explicit_source_passthrough():
    assert _resolve_provider("copernicus", VANCOUVER) == "copernicus"
    assert _resolve_provider("eudtm", SWEDEN) == "eudtm"        # honoured regardless of coverage/key
    with pytest.raises(ValueError, match="unknown --source"):
        _resolve_provider("nope", SWEDEN)


def test_auto_without_key_is_copernicus(monkeypatch):
    monkeypatch.delenv("OPENTOPOGRAPHY_API_KEY", raising=False)
    assert _resolve_provider("auto", SWEDEN) == "copernicus"    # Europe but no key → fallback
    assert _resolve_provider("auto", VANCOUVER) == "copernicus"


def test_auto_with_key_picks_eudtm_in_europe(monkeypatch):
    monkeypatch.setenv("OPENTOPOGRAPHY_API_KEY", "test-key")
    assert _resolve_provider("auto", SWEDEN) == "eudtm"         # Europe + key → bare-earth EU-DTM
    assert _resolve_provider("auto", VANCOUVER) == "copernicus"  # outside Europe → still copernicus


def test_dem_uri_keeps_urls():
    from pathlib import Path
    from duckosm.elevation import _dem_uri
    assert _dem_uri("https://example.org/dem.tif") == "https://example.org/dem.tif"
    assert _dem_uri("/vsicurl/https://example.org/dem.tif") == "/vsicurl/https://example.org/dem.tif"
    assert _dem_uri("dem.tif") == str(Path("dem.tif").resolve())

