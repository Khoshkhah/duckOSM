"""boundary.buffer_m — outward margin applied before clipping (auto-UTM, metres)."""
import json
from pathlib import Path

import duckdb
import pytest

from duckosm.importer import DuckOSM
from duckosm.config import Config, Boundary


def _square(tmp_path):
    sq = {"type": "FeatureCollection", "features": [{"type": "Feature", "properties": {},
          "geometry": {"type": "Polygon", "coordinates": [[
              [18.06, 59.31], [18.07, 59.31], [18.07, 59.32], [18.06, 59.32], [18.06, 59.31]]]}}]}
    p = tmp_path / "b.geojson"
    p.write_text(json.dumps(sq))
    return p


def _importer(path, buf):
    d = DuckOSM.__new__(DuckOSM)                 # bypass heavy __init__/validate for a unit test
    d.config = Config(boundary=Boundary(path=str(path), buffer_m=buf))
    d.con = duckdb.connect(); d.con.execute("LOAD spatial;")
    return d


def _area_m2(con, f):
    return con.execute(f"SELECT ST_Area(ST_Transform(geom, 'EPSG:4326','EPSG:32634', "
                       f"always_xy := true)) FROM ST_Read('{f}') LIMIT 1").fetchone()[0]


def test_buffer_zero_returns_raw_path(tmp_path):
    p = _square(tmp_path)
    assert _importer(p, 0)._effective_boundary_path() == str(p)


def test_buffer_grows_the_boundary(tmp_path):
    p = _square(tmp_path)
    d = _importer(p, 300)
    out = d._effective_boundary_path()
    assert out != str(p) and Path(out).exists()
    assert _area_m2(d.con, out) > _area_m2(d.con, p)          # buffered polygon is larger
    # ~300 m ring around a ~1.1 x 1.1 km square adds well over 1 km^2
    assert _area_m2(d.con, out) - _area_m2(d.con, p) > 1_000_000


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-q"]))
