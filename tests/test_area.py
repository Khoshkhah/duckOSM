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
