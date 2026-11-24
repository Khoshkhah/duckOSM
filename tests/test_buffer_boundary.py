"""boundary.buffer_m — outward margin applied before clipping (auto-UTM, metres).

The Rust build grows the boundary and writes `<name>.buffer<m>m.geojson` next to it. The tests build a tiny
area of the Monaco sample through `duckosm build -c`."""
import json
from pathlib import Path

import duckdb

from cli_run import invoke

ROOT = Path(__file__).resolve().parents[1]
PBF = ROOT / "data" / "sample" / "monaco.osm.pbf"


def _square(tmp_path):
    sq = {"type": "FeatureCollection", "features": [{"type": "Feature", "properties": {},
          "geometry": {"type": "Polygon", "coordinates": [[
              [7.41, 43.73], [7.42, 43.73], [7.42, 43.74], [7.41, 43.74], [7.41, 43.73]]]}}]}
    p = tmp_path / "b.geojson"
    p.write_text(json.dumps(sq))
    return p


def _build(tmp_path, boundary, buf):
    cfg = tmp_path / "c.yaml"
    cfg.write_text(
        f"name: t\noutput_path: {tmp_path / 'out'}\nsource: {{type: pbf, pbf_path: {PBF}}}\n"
        f"boundary: {{path: {boundary}, buffer_m: {buf}}}\nmodes: [driving]\n"
        "options: {build_graph: false, build_features: false, h3_indexing: false, bus_routes: false, levels: false}\n")
    r = invoke(["-c", cfg])
    assert r.exit_code == 0, r.output


def _area_m2(f):
    con = duckdb.connect()
    con.execute("LOAD spatial")
    return con.execute(f"SELECT ST_Area(ST_Transform(geom, 'EPSG:4326','EPSG:32632', "
                       f"always_xy := true)) FROM ST_Read('{f}') LIMIT 1").fetchone()[0]


def test_buffer_zero_writes_no_buffered_file(tmp_path):
    p = _square(tmp_path)
    _build(tmp_path, p, 0)
    assert not list(tmp_path.glob("*.buffer*m.geojson"))


def test_buffer_grows_the_boundary(tmp_path):
    p = _square(tmp_path)
    _build(tmp_path, p, 300)
    out = tmp_path / "b.buffer300m.geojson"
    assert out.exists()
    # a 300 m ring around a ~0.8 x 1.1 km square adds well over 0.8 km^2
    assert _area_m2(out) - _area_m2(p) > 800_000
