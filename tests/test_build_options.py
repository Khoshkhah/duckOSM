"""The build options of the SDK (duckosm._api.BUILD_OPTIONS) and config= (a file or a dict), on Monaco."""
import json
from pathlib import Path

import pytest

import duckosm
from duckosm import _api

DATA = Path(__file__).parent / "data"
PBF, BOUNDARY = DATA / "monaco.osm.pbf", DATA / "monaco.geojson"
FAST = {"features": False, "levels": False, "modes": ["driving"]}


def _build(tmp_path, name, **kw):
    return duckosm.build(pbf=PBF, boundary=BOUNDARY, output=tmp_path / f"{name}.duckdb", progress=False, **{**FAST, **kw})


def _has_bus(net):
    return any(t.startswith("bus.") for t in net.tables())


def test_options_and_config(tmp_path):
    plain = _build(tmp_path, "plain")
    assert _has_bus(plain) and "maxspeed_kmh" in set(plain.columns("driving.edges").column_name)
    pieces = _build(tmp_path, "pieces", speeds=False, bus_routes=False)   # the keywords
    cols = set(pieces.columns("driving.edges").column_name)
    assert "maxspeed_kmh" not in cols and "cost_s" not in cols and not _has_bus(pieces)        # speeds off: costs off
    as_dict = _build(tmp_path, "as_dict", config={"options": {"bus_routes": False}})          # a dict
    assert not _has_bus(as_dict)
    f = tmp_path / "c.json"                                                                   # a JSON file
    f.write_text(json.dumps({"options": {"bus_routes": False}}))
    assert not _has_bus(_build(tmp_path, "as_json", config=f))
    over = _build(tmp_path, "over", config={"options": {"bus_routes": False}}, bus_routes=True)  # a keyword wins
    assert _has_bus(over)
    assert list(tmp_path.glob("duckosm-config-*")) == [] and f.exists()                       # no file left over


def test_each_keyword_is_its_config_key(tmp_path):
    f = _api._build_config({"options": {"bus_routes": True}, "name": "x"},
                           {"costs": False, "restrictions": False, "bus_routes": False,
                            "connect_paths": False, "snap_m": 5.0})
    cfg = json.loads(Path(f).read_text())
    Path(f).unlink()
    assert cfg == {"name": "x", "options": {"calculate_costs": False, "extract_restrictions": False,
                                            "bus_routes": False}, "clip": {"connectivity_rescue": False, "connect_snap_m": 5.0}}
    assert _api._build_config("my.yaml", {}) == "my.yaml"                    # a file and no keyword: the file itself


def test_wrong_option_names_the_right_one(tmp_path):
    with pytest.raises(duckosm.DuckOSMError, match="did you mean 'bus_routes'"):
        _build(tmp_path, "x", bus_route=False)


def test_network_own_pbf_and_dem(tmp_path, monkeypatch):
    calls = []
    monkeypatch.setattr(_api, "to_elevation", lambda db, dem=None, **kw: calls.append((Path(db).name, dem)))
    net = duckosm.network(pbf=PBF, boundary=BOUNDARY, out=tmp_path, name="cut", modes=["driving"], features=False,
                          levels=False, bus_routes=False, dem="ground.tif", progress=False)       # own PBF and a boundary
    assert net.path.name == "cut.duckdb" and not _has_bus(net) and calls == [("cut.duckdb", "ground.tif")]
    whole = duckosm.network(pbf=DATA / "monaco_sample.osm.pbf", out=tmp_path, modes=["driving"], features=False,
                            levels=False, progress=False)                                         # no boundary: the file
    assert len(whole.driving.edges()) > 0
