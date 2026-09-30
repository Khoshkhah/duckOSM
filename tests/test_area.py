"""duckosm boundary / clip-pbf on the Monaco sample (offline: the PBF's own borders)."""
import json
import shutil
from pathlib import Path

import pytest

from duckosm.area import clip_pbf, find_boundary, write_boundary

PBF = Path(__file__).resolve().parents[1] / "data" / "sample" / "monaco.osm.pbf"
needs_gdal = pytest.mark.skipif(not shutil.which("ogr2ogr"), reason="ogr2ogr (GDAL) not installed")


@needs_gdal
def test_boundary_by_name_from_the_pbf():
    _, info = find_boundary("Monaco", pbf=PBF, offline=True)
    # two borders are called "Monaco": the exact-name rule then prefers the larger, the country
    assert (info["osm_id"], info["admin_level"]) == (1124039, 2)
    assert 2220322 in [o["osm_id"] for o in info["others"]]                 # listed, to pick with --osm-id
    _, info = find_boundary("monte carlo", pbf=PBF, offline=True)          # case, spaces, partial
    assert info["name"] == "Monte-Carlo"


@needs_gdal
def test_boundary_by_osm_id_and_not_found():
    _, info = find_boundary(pbf=PBF, osm_id=2220322, offline=True)          # the municipality
    assert info["admin_level"] == 8
    with pytest.raises(LookupError):
        find_boundary("Atlantis", pbf=PBF, offline=True)


@needs_gdal
@pytest.mark.skipif(not shutil.which("osmium"), reason="osmium not installed")
def test_clip_pbf_to_the_boundary(tmp_path):
    geom, info = find_boundary("Monaco", pbf=PBF, offline=True)
    b = write_boundary(geom, info, tmp_path / "monaco.geojson")
    assert json.loads(b.read_text())["features"][0]["properties"]["osm_id"] == 1124039
    out = clip_pbf(PBF, b, tmp_path / "monaco.osm.pbf")
    assert 0 < out.stat().st_size < PBF.stat().st_size                        # the French edges are cut


def test_db_named_like_a_schema_is_refused(tmp_path):
    """DuckDB names a database after its file: `driving.duckdb` / `mm.duckdb` make every
    `driving.edges` / `mm.edges` ambiguous. The CLI refuses such a file with a clear message."""
    import duckdb
    from click.testing import CliRunner
    from duckosm.cli import main
    from duckosm.utils import check_db_name
    for bad in ("driving.duckdb", "mm.duckdb", "Walking.duckdb"):
        with pytest.raises(ValueError, match="Rename it"):
            check_db_name(tmp_path / bad)
    check_db_name(tmp_path / "monaco.duckdb")
    db = tmp_path / "mm.duckdb"
    duckdb.connect(str(db)).close()
    r = CliRunner().invoke(main, ["multimodal", str(db)])
    assert r.exit_code != 0 and "Rename it" in r.output


def test_pbf_cut_is_keyed_on_boundary_and_pbf(tmp_path):
    """A changed boundary or a newer PBF must never reuse an old osmium cut (it silently built the
    old area): the cached file name carries a fingerprint of both."""
    import os
    from duckosm.importer import pbf_cut_path
    pbf, b1, b2 = tmp_path / "x.osm.pbf", tmp_path / "a.geojson", tmp_path / "b.geojson"
    pbf.write_bytes(b"pbf"); b1.write_text('{"a": 1}'); b2.write_text('{"a": 2}')
    p1 = pbf_cut_path("pbf", "area", "complete_ways", pbf, b1)
    assert p1 == pbf_cut_path("pbf", "area", "complete_ways", pbf, b1)          # stable
    assert p1 != pbf_cut_path("pbf", "area", "complete_ways", pbf, b2)          # new boundary
    assert p1 != pbf_cut_path("pbf", "area", "smart", pbf, b1)                  # other strategy
    os.utime(pbf, ns=(1, 1))
    assert p1 != pbf_cut_path("pbf", "area", "complete_ways", pbf, b1)          # newer PBF
    assert p1.name.startswith("area.complete_ways.") and p1.name.endswith(".osm.pbf")
