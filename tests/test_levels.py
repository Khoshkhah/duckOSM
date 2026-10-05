"""`duckosm levels`: the drawing order of the roads, stored in the visualization schema (docs/design/levels.md).

Needs roadstyle 0.13.1 or later (the `levels` extra)."""
import os
from pathlib import Path

import duckdb
import pytest
from click.testing import CliRunner

from duckosm import Config, DuckOSM
from duckosm.cli import main

rs = pytest.importorskip("roadstyle")
if not hasattr(rs, "save_levels"):
    pytest.skip("roadstyle 0.13 or later is needed", allow_module_level=True)

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


def test_levels_command_writes_the_visualization_schema(monaco, tmp_path):
    db = tmp_path / "copy.duckdb"
    db.write_bytes(monaco.read_bytes())
    r = CliRunner().invoke(main, ["levels", str(db)])
    assert r.exit_code == 0, r.output
    assert "wrote visualization.edge_levels" in r.output and "0 stack pairs given up" in r.output
    con = duckdb.connect(str(db), read_only=True)
    con.execute("LOAD spatial")
    n = con.execute("SELECT count(*), count(DISTINCT edge_id) FROM visualization.edge_levels").fetchone()
    assert n[0] == n[1] == _all_edge_ids(con)
    types = dict(con.execute("SELECT column_name, column_type FROM (DESCRIBE visualization.edge_levels)").fetchall())
    assert types == {"edge_id": "BIGINT", "casing_start": "INTEGER", "casing_level": "INTEGER", "casing_end": "INTEGER", "fill_level": "INTEGER"}
    meta = con.execute("SELECT * FROM visualization.edge_levels_meta").df().iloc[0]
    assert (meta["band_source"], meta["order_source"], int(meta["n_edges"])) == ("band", "class", n[0])
    from duckosm.levels import BAND_RULE
    assert meta["band_rule"] == BAND_RULE == "tags"                                   # the rule of the band the numbers were solved from: mapstyle refuses another
    # the reader accepts the parameters it was computed with and refuses others
    from duckosm.levels import load_roads
    roads = load_roads(db)
    assert len(rs.load_levels(con, roads, band_col="band", order="class")) == len(roads)
    with pytest.raises(ValueError, match="other parameters"):
        rs.load_levels(con, roads, band_col="band", order=None)
    info = CliRunner().invoke(main, ["info", str(db)])
    assert "visualization (drawing order)" in info.output


def test_levels_command_errors_say_what_is_wrong(tmp_path):
    db = tmp_path / "empty.duckdb"
    duckdb.connect(str(db)).close()
    r = CliRunner().invoke(main, ["levels", str(db)])
    assert r.exit_code != 0 and "no <mode>.edges table" in r.output


def test_min_positions_option(monaco, tmp_path):
    """--min-positions is on by default and stored; --no-min-positions leaves it out; no more positions are used with it."""
    import pandas as pd

    def run(extra):
        db = tmp_path / f"copy{len(extra)}.duckdb"
        db.write_bytes(monaco.read_bytes())
        r = CliRunner().invoke(main, ["levels", str(db), *extra])
        assert r.exit_code == 0, r.output
        con = duckdb.connect(str(db), read_only=True)
        meta = con.execute("SELECT * FROM visualization.edge_levels_meta").df().iloc[0]
        values = con.execute("SELECT count(DISTINCT v) FROM (SELECT unnest([casing_start, casing_level, casing_end, fill_level]) AS v FROM visualization.edge_levels)").fetchone()[0]
        return meta, values
    short, n_short = run([])
    plain, n_plain = run(["--no-min-positions"])
    assert short["min_positions"] == True and pd.isna(plain["min_positions"]) and n_short <= n_plain          # noqa: E712


def test_a_reverse_row_has_the_numbers_of_its_road_with_its_heads_swapped(monaco, tmp_path):
    """docs/design/levels.md, "A road and its reverse row": the optimization is for the roads, a reverse row takes its road's numbers with the two heads swapped (its line runs the other way), and the table keeps one row for every edge_id."""
    from duckosm.levels import load_roads, reverse_rows
    db = tmp_path / "copy.duckdb"
    db.write_bytes(monaco.read_bytes())
    assert CliRunner().invoke(main, ["levels", str(db)]).exit_code == 0
    twin = reverse_rows(db)
    assert twin and not set(twin) & set(twin.values())                      # a road is never a reverse row
    con = duckdb.connect(str(db), read_only=True)
    t = con.execute("SELECT * FROM visualization.edge_levels").df().set_index("edge_id")
    assert len(t) == len(load_roads(db)) and t.index.is_unique
    cols = ["casing_start", "casing_level", "casing_end", "fill_level"]
    for rev, road in twin.items():
        s, m, e, f = t.loc[road, cols]
        assert list(t.loc[rev, cols]) == [e, m, s, f]
