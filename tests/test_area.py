"""duckosm boundary / clip-pbf on the Monaco sample (offline: the PBF's own borders)."""
import json
from pathlib import Path

import pytest

from duckosm.area import clip_pbf, find_boundary, write_boundary

PBF = Path(__file__).resolve().parents[1] / "data" / "sample" / "monaco.osm.pbf"


def test_boundary_by_name_from_the_pbf():
    _, info = find_boundary("Monaco", pbf=PBF, offline=True)
    # two borders are called "Monaco": the exact-name rule then prefers the larger, the country
    assert (info["osm_id"], info["admin_level"]) == (1124039, 2)
    _, info = find_boundary("monte carlo", pbf=PBF, offline=True)          # case, spaces, partial
    assert info["name"] == "Monte-Carlo"


def test_boundary_by_osm_id_and_not_found():
    _, info = find_boundary(pbf=PBF, osm_id=2220322, offline=True)          # the municipality
    assert info["admin_level"] == 8
    with pytest.raises(LookupError):
        find_boundary("Atlantis", pbf=PBF, offline=True)


def test_clip_pbf_to_the_boundary(tmp_path):
    geom, info = find_boundary("Monaco", pbf=PBF, offline=True)
    b = write_boundary(geom, info, tmp_path / "monaco.geojson")
    assert json.loads(b.read_text())["features"][0]["properties"]["osm_id"] == 1124039
    out = clip_pbf(PBF, b, tmp_path / "monaco.osm.pbf")
    assert 0 < out.stat().st_size < PBF.stat().st_size                        # the French edges are cut


def test_db_named_like_a_schema_is_refused(tmp_path):
    """DuckDB names a database after its file: `driving.duckdb` / `mm.duckdb` make every
    `driving.edges` / `mm.edges` ambiguous. The CLI refuses such a file with a clear message."""
    import subprocess
    import sys

    import duckdb
    db = tmp_path / "mm.duckdb"
    duckdb.connect(str(db)).close()
    r = subprocess.run([sys.executable, "-m", "duckosm", "multimodal", str(db)], capture_output=True, text=True)
    assert r.returncode != 0 and "Rename it" in r.stderr


def test_boundary_from_bbox_h3_or_place(tmp_path, monkeypatch):
    """boundary.bbox / h3_cell / place become a GeoJSON file next to the output; path wins."""
    import json
    from duckosm import Config
    from duckosm.config import Boundary
    pbf = tmp_path / "x.osm.pbf"
    pbf.write_bytes(b"x")

    def cfg(**b):
        return Config(name="area", pbf_path=str(pbf), output_path=str(tmp_path / "area.duckdb"),
                      boundary=Boundary(**b))

    c = cfg(bbox=[7.40, 43.72, 7.44, 43.75])
    c.materialize_boundary()
    ring = json.loads(open(c.boundary.path).read())["features"][0]["geometry"]["coordinates"][0]
    assert c.boundary.path.endswith("area.boundary.geojson") and ring[0] == [7.40, 43.72] and len(ring) == 5
    with pytest.raises(ValueError, match="west, south, east, north"):
        cfg(bbox=[7.44, 43.72, 7.40, 43.75]).materialize_boundary()

    c = cfg(h3_cell="883969a403fffff")
    c.materialize_boundary()
    assert len(json.loads(open(c.boundary.path).read())["features"][0]["geometry"]["coordinates"][0]) == 7
    with pytest.raises(ValueError, match="H3"):
        cfg(h3_cell="nope").materialize_boundary()

    monkeypatch.setattr("duckosm.area.find_boundary",
                        lambda name, pbf=None: ('{"type": "Point", "coordinates": [7.42, 43.73]}',
                                                {"name": name, "source": "test"}))
    c = cfg(place="Monaco")
    c.materialize_boundary()
    assert json.loads(open(c.boundary.path).read())["features"][0]["properties"]["name"] == "Monaco"

    c = cfg(path="given.geojson", bbox=[7.40, 43.72, 7.44, 43.75])
    c.materialize_boundary()
    assert c.boundary.path == "given.geojson"                                   # path wins


def test_unknown_config_key_warns(tmp_path, caplog):
    from duckosm import Config
    y = tmp_path / "c.yaml"
    y.write_text("name: a\nclip:\n  strongly_connected: true\n  keep_largest_component: true\n")
    with caplog.at_level("WARNING", logger="duckosm"):
        c = Config.from_yaml(str(y))
    assert "strongly_connected" in caplog.text and c.clip.keep_largest_component


def test_misspelled_section_warns(tmp_path, caplog):
    """A misspelled section (`clipp:`) once dropped its settings without a word."""
    from duckosm import Config
    y = tmp_path / "c.yaml"
    y.write_text("name: a\nclipp:\n  predicate: within\npbf_path: x.pbf\n")
    with caplog.at_level("WARNING", logger="duckosm"):
        Config.from_yaml(str(y))
    assert "clipp" in caplog.text and "pbf_path" not in caplog.text     # the flat shorthand is known


@pytest.mark.parametrize("yaml_text, bad", [
    ("modes: [driving, flying]", "flying"),
    ("options:\n  clip_strategy: smrt", "smrt"),
    ("clip:\n  predicate: inside", "inside")])
def test_wrong_choice_values_stop_the_build(tmp_path, yaml_text, bad):
    """A typo in a value with fixed choices stops the build (an unknown clip.predicate once quietly
    became intersects)."""
    from duckosm import Config
    y = tmp_path / "c.yaml"
    y.write_text(f"name: a\n{yaml_text}\n")
    with pytest.raises(ValueError, match=bad):
        Config.from_yaml(str(y)).validate()

