"""`duckosm levels`: the drawing order of the roads, stored in the visualization schema (docs/design/levels.md).

Needs roadstyle 0.17 or later (the `levels` extra: its level areas)."""
import os
from pathlib import Path

import duckdb
import pytest
from click.testing import CliRunner

from duckosm import Config, DuckOSM
from duckosm.cli import main

rs = pytest.importorskip("roadstyle")
if not hasattr(rs, "save_area_levels"):
    pytest.skip("roadstyle 0.17 or later is needed", allow_module_level=True)

ROOT = Path(__file__).resolve().parents[1]


@pytest.fixture(scope="module")
def monaco(tmp_path_factory):
    d = tmp_path_factory.mktemp("monaco_levels")
    cfg = Config.from_yaml(str(ROOT / "config" / "sample_monaco.yaml"))
    cfg.source.pbf_path = str(ROOT / cfg.source.pbf_path)
    cfg.boundary.path = str(ROOT / cfg.boundary.path)
    cfg.output_path = str(d)
    cwd = os.getcwd()
    os.chdir(d)
    try:
        DuckOSM(cfg).run()
    finally:
        os.chdir(cwd)
    return d / "monaco.duckdb"


def _all_edge_ids(con):
    parts = [f"SELECT edge_id FROM {m}.{t}" for m in ("driving", "walking", "cycling") for t in ("edges", "private_edges")]
    return con.execute(f"SELECT count(DISTINCT edge_id) FROM ({' UNION ALL '.join(parts)})").fetchone()[0]


def test_the_roads_of_all_modes_with_their_bands(monaco):
    from duckosm.levels import load_roads
    roads = load_roads(monaco)
    con = duckdb.connect(str(monaco), read_only=True)
    assert len(roads) == roads["edge_id"].nunique() == _all_edge_ids(con)             # one row per edge_id, every mode
    assert roads["edge_id"].is_monotonic_increasing
    from duckosm.crossings import _level
    assert (roads["band"].to_numpy() == [_level(ly, br, tn) for ly, br, tn in zip(roads["layer"], roads["bridge"], roads["tunnel"], strict=True)]).all()   # the tags only: a sidewalk or a crossing is on its street's floor
    assert (roads["walk_type"] == "sidewalk").any() and (roads["walk_type"] == "crossing").any()
    assert roads["highway"].isna().any()                                               # a ferry with no highway is a road too


def test_levels_command_makes_the_area_and_writes_the_db(monaco, tmp_path):
    """duckosm levels DB: one level area for all modes next to the db (DB.levels), solved by roadstyle, the result in visualization.edge_levels
    with every edge's ends; roadstyle's reader takes it for the db's roads."""
    db = tmp_path / "copy.duckdb"
    db.write_bytes(monaco.read_bytes())
    r = CliRunner().invoke(main, ["levels", str(db)])
    assert r.exit_code == 0, r.output
    assert "wrote visualization.edge_levels" in r.output and "0 stack pairs given up" in r.output
    area = tmp_path / "copy.levels"
    assert {"roads.parquet", "pairs.csv", "edits.csv", "area.json", "levels.csv"} <= {p.name for p in area.iterdir()}
    con = duckdb.connect(str(db), read_only=True)
    con.execute("LOAD spatial")
    n = con.execute("SELECT count(*), count(DISTINCT edge_id) FROM visualization.edge_levels").fetchone()
    assert n[0] == n[1] == _all_edge_ids(con)
    cols = {c for c, in con.execute("SELECT column_name FROM (DESCRIBE visualization.edge_levels)").fetchall()}
    assert {"edge_id", "casing_start", "casing_level", "casing_end", "fill_level", "head_start_m", "head_end_m", "cap_start", "cap_end"} <= cols
    from duckosm.levels import load_roads
    roads = load_roads(db)
    assert len(rs.load_area_levels(con, roads)) == len(roads)
    info = CliRunner().invoke(main, ["info", str(db)])
    assert "visualization (drawing order)" in info.output


def test_your_heads_survive_a_new_run(monaco, tmp_path):
    """heads.csv / caps.csv / edits.csv in the area are yours: a new run keeps them and the db gets them."""
    db = tmp_path / "copy.duckdb"
    db.write_bytes(monaco.read_bytes())
    assert CliRunner().invoke(main, ["levels", str(db)]).exit_code == 0
    import pandas as pd
    road = pd.read_parquet(tmp_path / "copy.levels" / "roads.parquet")["road"].iloc[0]
    (tmp_path / "copy.levels" / "heads.csv").write_text(f"road,start_m,end_m\n{road},2.5,\n")
    assert CliRunner().invoke(main, ["levels", str(db)]).exit_code == 0
    assert str(road) in (tmp_path / "copy.levels" / "heads.csv").read_text()
    con = duckdb.connect(str(db), read_only=True)
    assert con.execute("SELECT head_start_m FROM visualization.edge_levels WHERE edge_id = ?", [int(road)]).fetchone()[0] == 2.5


def test_levels_command_errors_say_what_is_wrong(tmp_path):
    db = tmp_path / "empty.duckdb"
    duckdb.connect(str(db)).close()
    r = CliRunner().invoke(main, ["levels", str(db)])
    assert r.exit_code != 0 and "no <mode>.edges table" in r.output
