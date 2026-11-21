"""duckosm.fixes on Monaco: Fixes, net.fixes, net.rebuild, net.diff (docs/proposals/python-sdk.md section 5.9)."""
import shutil
from pathlib import Path

import pytest
import yaml

import duckosm
from duckosm.fixes import Fixes

DATA = Path(__file__).parent / "data"


def test_fixes_file(tmp_path):
    f = Fixes()
    f.set(way=1, oneway=True, maxspeed=30).set(way=1, lanes=2).exclude(way=2, modes="driving").remove(way=3)
    assert not list(tmp_path.iterdir()), "a change writes no file"
    with pytest.raises(duckosm.DuckOSMError, match="needs a path"):
        f.save()
    f.save(tmp_path / "osm_fixes.yaml")
    rules = yaml.safe_load(f.path.read_text())["overrides"]
    assert rules == [{"osm_id": 1, "oneway": True, "tags": {"maxspeed": "30"}, "lanes": 2},
                     {"osm_id": 2, "exclude_modes": ["driving"]}, {"osm_id": 3, "remove": True}]
    assert len(Fixes(f.path)) == 3 and "<table" in f._repr_html_()
    f.drop(way=3).save()                                            # a later save() writes the same file again
    assert list(Fixes(f.path).rules().osm_id) == [1, 2]


def test_the_existing_file_keeps_its_other_keys(tmp_path):
    shutil.copy(DATA / "osm_fixes.yaml", tmp_path / "f.yaml")
    before = yaml.safe_load((tmp_path / "f.yaml").read_text())
    text = (tmp_path / "f.yaml").read_text()
    f = Fixes(load=tmp_path / "f.yaml").set(way=4392632, note="checked")
    assert (tmp_path / "f.yaml").read_text() == text, "a load and a change do not write the loaded file"
    f.save(tmp_path / "v2.yaml")
    assert (tmp_path / "f.yaml").read_text() == text, "a save to another file keeps the loaded file"
    after = yaml.safe_load((tmp_path / "v2.yaml").read_text())
    assert len(after["overrides"]) == len(before["overrides"]) and set(after) == set(before)


def test_rebuild_and_diff(tmp_path):
    shutil.copy(DATA / "monaco.osm.pbf", tmp_path / "monaco.osm.pbf")
    shutil.copy(DATA / "monaco.geojson", tmp_path / "monaco.geojson")
    net = duckosm.build(pbf=tmp_path / "monaco.osm.pbf", boundary=tmp_path / "monaco.geojson", modes=["driving"],
                        output=tmp_path / "monaco.duckdb", progress=False)
    assert net.fixes is None
    two_way = net.sql("SELECT osm_id FROM driving.edges WHERE NOT oneway AND osm_id > 0 ORDER BY length_m DESC LIMIT 1").osm_id[0]
    slow = net.sql(f"SELECT osm_id FROM driving.edges WHERE osm_id NOT IN ({two_way}) AND osm_id > 0 "
                   "ORDER BY length_m DESC LIMIT 1").osm_id[0]
    fixes = Fixes().set(way=int(two_way), oneway=True).set(way=int(slow), maxspeed=10)
    with pytest.raises(duckosm.DuckOSMError, match="save the fixes first"):
        net.rebuild(fixes=fixes, progress=False)
    fixed = net.rebuild(fixes=fixes.save(tmp_path / "osm_fixes.yaml"), progress=False)
    assert fixed.path.name == "monaco_fixed.duckdb" and fixed.fixes is not None and len(fixed.fixes) == 2
    d = net.diff(fixed)
    removed = d[d.change == "removed"]
    assert len(removed) > 0 and set(net.sql(f"SELECT edge_id FROM driving.edges WHERE osm_id = {two_way} AND is_reverse")
                                    .edge_id) == set(removed.edge_id)
    speed = d[d.change == "changed"]
    assert set(net.sql(f"SELECT edge_id FROM driving.edges WHERE osm_id = {slow}").edge_id) <= set(speed.edge_id)
    assert all("maxspeed_kmh" in c for c, e in zip(speed.changed, speed.edge_id)
               if e in set(net.sql(f"SELECT edge_id FROM driving.edges WHERE osm_id = {slow}").edge_id))
    assert len(net.diff(net)) == 0


def test_edit_serves_the_page(tmp_path):
    import socket
    import urllib.request
    net = duckosm.build(pbf=DATA / "monaco.osm.pbf", boundary=DATA / "monaco.geojson", modes=["driving"],
                        output=tmp_path / "monaco.duckdb", progress=False)
    with socket.socket() as s:                                        # a free port
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    url = net.edit(port=port)
    assert str(port) in url and urllib.request.urlopen(url, timeout=10).status == 200
    # the library chooses no fixes file; fixes= is the old name of load= and save=
    info = urllib.request.urlopen(url + "api/info", timeout=10).read().decode()
    assert '"load":null' in info and '"save":null' in info, info
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    with pytest.warns(DeprecationWarning):
        url = net.edit(fixes=tmp_path / "f.yaml", port=port)
    info = urllib.request.urlopen(url + "api/info", timeout=10).read().decode()
    assert '"load":' in info and '"load":null' not in info and '"save":null' not in info, info
