"""Saving (docs/proposals/python-sdk.md): a network that you build goes to duckOSM's work folder; net.save, area.save
and network(path=...) put it where you want it; clear_work deletes the rest. Monaco, no download."""
from pathlib import Path

import pytest

import duckosm

DATA = Path(__file__).parent / "data"
PBF, BOUNDARY = DATA / "monaco.osm.pbf", DATA / "monaco.geojson"
FAST = {"modes": ["driving"], "features": False, "levels": False, "progress": False}


@pytest.fixture(autouse=True)
def work(tmp_path, monkeypatch):
    monkeypatch.setenv("DUCKOSM_CACHE", str(tmp_path / "cache"))
    monkeypatch.chdir(tmp_path)                        # nothing may land in the folder of the user
    return tmp_path


def test_a_build_goes_to_the_work_folder_and_save_keeps_it(work):
    net = duckosm.network(pbf=PBF, boundary=BOUNDARY, **FAST)
    assert not net.saved and duckosm.work_dir() in net.path.parents
    assert "not saved" in net._repr_html_()
    assert sorted(p.name for p in work.iterdir()) == ["cache"]          # the folder of the user stays clean
    net.path.with_name(net.path.name.split(".")[0] + ".osm.pbf").write_bytes(b"pbf")   # a companion file
    kept = net.save("out/monaco.duckdb")
    assert kept.saved and kept.path == Path("out/monaco.duckdb") and len(kept.driving.edges()) == len(net.driving.edges())
    assert Path("out/monaco.osm.pbf").read_bytes() == b"pbf"            # the companions go with it, under the new name
    assert "not saved" not in kept._repr_html_()
    with pytest.raises(duckosm.DuckOSMError, match="exists"):
        net.save("out/monaco.duckdb")
    assert net.save("out/", overwrite=True).path == Path("out") / net.path.name   # a folder: <folder>/<name>.duckdb


def test_path_writes_the_network_where_you_say(work):
    net = duckosm.network(pbf=PBF, boundary=BOUNDARY, path="nets/monaco.duckdb", **FAST)
    assert net.path == Path("nets/monaco.duckdb") and net.saved


def test_area_save_and_clear_work(work):
    area = duckosm.Area(PBF, BOUNDARY)
    kept = area.save("my_area", name="monaco")
    assert kept.pbf == Path("my_area/monaco.osm.pbf") and kept.boundary.read_text() == BOUNDARY.read_text()
    duckosm.network(pbf=PBF, boundary=BOUNDARY, **FAST)
    assert duckosm.clear_work(days=1) == 0                               # a new folder is younger than one day
    assert duckosm.clear_work() == 1 and not any(duckosm.work_dir().iterdir())


def test_cards_show_the_home_folder_as_tilde(work, monkeypatch):
    monkeypatch.setenv("HOME", str(work))                      # the work folder is now under the home folder
    monkeypatch.setenv("DUCKOSM_CACHE", str(work / ".cache" / "duckosm"))
    net = duckosm.network(pbf=PBF, boundary=BOUNDARY, **FAST)
    assert str(work) not in repr(net) and str(work) not in net._repr_html_() and "~/.cache/duckosm/work/" in repr(net)
    lines = []
    duckosm.network(pbf=PBF, boundary=BOUNDARY, **{**FAST, "progress": lines.append})
    from duckosm._progress import shown
    assert shown(str(work / "x.duckdb")) == "~/x.duckdb"
