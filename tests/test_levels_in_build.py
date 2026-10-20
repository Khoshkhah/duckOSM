"""The build makes visualization.edge_levels as its last step, and a rebuild keeps your edits (docs/design/levels_in_build.md)."""
import logging
import os
from pathlib import Path

import duckdb
import pytest

from duckosm import Config, DuckOSM

rs = pytest.importorskip("roadstyle")
pytest.importorskip("geopandas")
if not hasattr(rs, "restore_area_inputs"):
    pytest.skip("needs a roadstyle that keeps the level edits in the database (visualization.level_*)", allow_module_level=True)

ROOT = Path(__file__).resolve().parents[1]


def _config(out, levels=True):
    cfg = Config.from_yaml(str(ROOT / "config" / "sample_monaco.yaml"))
    cfg.source.pbf_path = str(ROOT / (cfg.source.pbf_path or ""))
    cfg.boundary.path = str(ROOT / (cfg.boundary.path or ""))
    if cfg.osm_overrides:
        cfg.osm_overrides = str(ROOT / cfg.osm_overrides)
    cfg.output_path = str(out)
    cfg.options.levels = levels
    cfg.viz.enabled = False
    cfg.report.enabled = False
    return cfg


def _build(cfg, folder):
    cwd = Path.cwd()
    os.chdir(folder)                                   # keep reports etc. out of the repo
    try:
        DuckOSM(cfg).run()
    finally:
        os.chdir(cwd)
    return Path(cfg.output_path) / "monaco.duckdb"


@pytest.fixture(scope="module")
def first(tmp_path_factory):
    out = tmp_path_factory.mktemp("levels_build")
    return _build(_config(out), out)


def _q(db, sql):
    con = duckdb.connect(str(db), read_only=True)
    try:
        return con.execute(sql).fetchall()
    finally:
        con.close()


def test_the_build_makes_the_drawing_order(first):
    n_levels, = _q(first, "SELECT count(*) FROM visualization.edge_levels")[0]
    n_meta, = _q(first, "SELECT n_edges FROM visualization.edge_levels_meta")[0]
    ids = {r[0] for m in ("driving", "walking", "cycling") for r in _q(first, f"SELECT edge_id FROM {m}.edges UNION SELECT edge_id FROM {m}.private_edges")}
    assert n_levels == n_meta == len(ids) > 1000          # one row for every road of every mode, and the meta says how many


def test_a_rebuild_keeps_your_edits_and_ignores_the_roads_that_are_gone(first, tmp_path, caplog):
    a_road, = _q(first, "SELECT edge_id FROM driving.edges ORDER BY edge_id LIMIT 1")[0]
    gone = 123456789                                       # a road that is not in the file
    con = duckdb.connect(str(first))                       # what the editor does at each Apply: the tables are the master
    con.execute(f"CREATE OR REPLACE TABLE visualization.level_heads AS SELECT * FROM (VALUES ({a_road}::BIGINT, '12', '5'), ({gone}::BIGINT, '9', '9')) t(road, start_m, end_m)")
    con.close()
    out = tmp_path / "again"
    out.mkdir()
    cfg = _config(out)
    cfg.output_path = str(first.parent)                    # the same place: the build replaces the file
    with caplog.at_level(logging.WARNING):
        db = _build(cfg, out)
    assert db == first
    assert _q(db, "SELECT road, start_m, end_m FROM visualization.level_heads") == [(a_road, "12", "5")]      # your row is in the new file
    assert [r[0] for r in _q(db, "SELECT kind FROM visualization.level_unmatched")] == ["level_heads"]        # the gone road's row is kept apart
    assert "not in this file" in caplog.text
    assert _q(db, f"SELECT head_start_m FROM visualization.edge_levels WHERE edge_id = {a_road}") == [(12.0,)]  # the solve used it


def test_a_road_that_comes_back_is_used_again(first):
    """The unmatched rows are tried at the next build: put the road in, the row is used."""
    a_road, = _q(first, "SELECT edge_id FROM walking.edges ORDER BY edge_id DESC LIMIT 1")[0]
    con = duckdb.connect(str(first))
    con.execute("DELETE FROM visualization.level_heads")
    con.execute("DROP TABLE IF EXISTS visualization.level_unmatched")
    con.execute(f"CREATE TABLE visualization.level_unmatched AS SELECT 'level_heads' AS kind, '{{\"road\": \"{a_road}\", \"start_m\": \"7\", \"end_m\": \"5\"}}' AS row")
    con.close()
    from duckosm.levels import make_and_solve
    res = make_and_solve(first)
    assert res["unmatched"] == 0
    assert "level_unmatched" not in {r[0] for r in _q(first, "SELECT table_name FROM information_schema.tables WHERE table_schema = 'visualization'")}
    assert _q(first, f"SELECT start_m FROM visualization.level_heads WHERE road = {a_road}") == [("7",)]


def test_the_switch_turns_the_step_off(tmp_path):
    db = _build(_config(tmp_path, levels=False), tmp_path)
    tables = {r[0] for r in _q(db, "SELECT table_name FROM information_schema.tables WHERE table_schema = 'visualization'")}
    assert "edge_levels" not in tables


def test_a_clip_takes_the_edits_of_its_parent(first, tmp_path):
    a_road, = _q(first, "SELECT edge_id FROM driving.edges ORDER BY edge_id LIMIT 1")[0]
    con = duckdb.connect(str(first))
    con.execute(f"CREATE OR REPLACE TABLE visualization.level_heads AS SELECT * FROM (VALUES ({a_road}::BIGINT, '12', '5'), (123456789::BIGINT, '9', '9')) t(road, start_m, end_m)")
    con.execute("DROP TABLE IF EXISTS visualization.level_unmatched")
    con.close()
    cfg = Config.from_args(pbf_path="", output_path=str(tmp_path / "clip.duckdb"), name="clip",
                           boundary_path=str(ROOT / "data" / "sample" / "monaco.geojson"), source_db=str(first),
                           build_graph=True, h3_indexing=False)
    cfg.viz.enabled = False
    cfg.report.enabled = False
    os.chdir(tmp_path)
    try:
        DuckOSM(cfg).run()
    finally:
        os.chdir(ROOT)
    clip = tmp_path / "clip.duckdb"
    assert _q(clip, "SELECT road, start_m FROM visualization.level_heads") == [(a_road, "12")]     # the road it has: kept
    tables = {r[0] for r in _q(clip, "SELECT table_name FROM information_schema.tables WHERE table_schema = 'visualization'")}
    assert "level_unmatched" not in tables                                                          # the parent's other rows: not kept


# ---- duckosm levels-edit -------------------------------------------------------------------------

def test_levels_edit_prepares_the_area_from_the_tables(first, tmp_path):
    from duckosm.levels import is_current
    from duckosm.levels_edit import prepare
    a_road, = _q(first, "SELECT edge_id FROM driving.edges ORDER BY edge_id LIMIT 1")[0]
    con = duckdb.connect(str(first))
    con.execute(f"CREATE OR REPLACE TABLE visualization.level_heads AS SELECT * FROM (VALUES ({a_road}::BIGINT, '11', '5')) t(road, start_m, end_m)")
    con.close()
    area = prepare(first, tmp_path / "work.levels")                  # a new folder: its files come from the tables, and it is solved
    assert (area / "heads.csv").read_text().splitlines() == ["road,start_m,end_m", f"{a_road},11,5"]
    assert is_current(first, area)
    (area / "heads.csv").write_text("road,start_m,end_m\n")           # a file that differs from the table: the table wins, the file is kept
    prepare(first, area)
    assert (area / "heads.csv").read_text().splitlines()[1] == f"{a_road},11,5"
    assert (area / "heads.csv.before").read_text() == "road,start_m,end_m\n"


def test_levels_edit_serves_the_editor_with_mapstyle_layers(first, tmp_path):
    pytest.importorskip("mapstyle")
    import socket
    import threading
    import time
    import urllib.request

    from duckosm.levels_edit import _access_colours, items_for, prepare
    from roadstyle.level_editor import serve
    items = items_for(first)
    import pandas as pd
    draw = pd.DataFrame({"edge": [1, 2]})
    overlays, kw = items(draw)
    assert overlays and all(o.visible and o.popup == [] for o in overlays)                    # water, land, buildings, …: decoration
    assert kw["basemap"] == "blank" and list(draw["access"]) == [None, None]                  # every road has an access column
    area = prepare(first, tmp_path / "work2.levels")
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    def run():
        with _access_colours(True):
            serve(area, port, items=items)
    threading.Thread(target=run, daemon=True).start()
    for _ in range(60):
        try:
            page = urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=60).read().decode()
            break
        except OSError:
            time.sleep(1)
    else:
        pytest.fail("the editor did not answer")
    assert "Level editor" in page and "buildings" in page
    assert "#9db8d9" in page and "window.rsColor=function" in page                           # the restricted roads are painted
