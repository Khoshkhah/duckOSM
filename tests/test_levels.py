"""`duckosm levels`: the drawing order of the roads, stored in the visualization schema (docs/design/levels.md).

Needs roadstyle 0.17 or later (the `levels` extra: its level areas)."""
import os
from pathlib import Path

import duckdb
import pytest

from cli_run import invoke
from duckosm import Config, DuckOSM

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
    if cfg.osm_overrides:
        cfg.osm_overrides = str(ROOT / cfg.osm_overrides)     # the sample's fixes file is relative to the repo too
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


def _roads(db, tmp_path):
    """The level roads of a db: `duckosm levels` writes them to the area folder next to the copy (`<name>.levels/roads.parquet`)."""
    import pandas as pd
    copy = tmp_path / "roads.duckdb"
    copy.write_bytes(db.read_bytes())
    assert invoke(["levels", str(copy)]).exit_code == 0
    return pd.read_parquet(tmp_path / "roads.levels" / "roads.parquet")


def test_the_roads_of_all_modes_with_their_bands(monaco, tmp_path):
    roads = _roads(monaco, tmp_path)
    con = duckdb.connect(str(monaco), read_only=True)
    ids = [e for col in ("edges", "reversed") for es in roads[col] for e in es]        # a road is an edge and its reverse
    assert len(ids) == len(set(ids)) == _all_edge_ids(con)                             # every edge_id of every mode, once
    assert roads["road"].is_unique
    bare = roads["layer"].isna() & roads["bridge"].isna() & roads["tunnel"].isna()
    assert (roads.loc[bare, "band"] == 0).all() and (roads["band"] != 0).any()         # the tags only: a sidewalk or a crossing is on its street's floor
    assert roads["highway"].notna().all() and (roads["highway"] == "ferry").any()      # a ferry is a road too, with its own class
    assert roads["edge_ref"].notna().all()
    modes = set(roads["modes"].dropna())                                               # who may use it: the level editor shows it
    assert "walking" in modes and any(m.startswith("driving + walking + cycling") for m in modes)   # (+ bus where a bus line runs)


def test_levels_command_makes_the_area_and_writes_the_db(monaco, tmp_path):
    """duckosm levels DB: one level area for all modes next to the db (DB.levels), solved by roadstyle, the result in visualization.edge_levels
    with every edge's ends; roadstyle's reader takes it for the db's roads."""
    db = tmp_path / "copy.duckdb"
    db.write_bytes(monaco.read_bytes())
    r = invoke(["levels", str(db)])
    assert r.exit_code == 0, r.output
    assert "(visualization.edge_levels)" in r.output and "0 given up" in r.output
    area = tmp_path / "copy.levels"
    assert {"roads.parquet", "pairs.csv", "edits.csv", "area.json", "levels.csv"} <= {p.name for p in area.iterdir()}
    con = duckdb.connect(str(db), read_only=True)
    con.execute("LOAD spatial")
    n = con.execute("SELECT count(*), count(DISTINCT edge_id) FROM visualization.edge_levels").fetchone()
    assert n[0] == n[1] == _all_edge_ids(con)
    cols = {c for c, in con.execute("SELECT column_name FROM (DESCRIBE visualization.edge_levels)").fetchall()}
    assert {"edge_id", "casing_start", "casing_level", "casing_end", "fill_level", "head_start_m", "head_end_m", "cap_start", "cap_end"} <= cols
    import pandas as pd
    roads = pd.read_parquet(area / "roads.parquet")
    assert n[0] == sum(len(a) + len(b) for a, b in zip(roads["edges"], roads["reversed"]))      # an end row for every edge of the roads
    info = invoke(["info", str(db)])
    assert "visualization (drawing order)" in info.output


def test_your_heads_survive_a_new_run(monaco, tmp_path):
    """visualization.level_heads is yours (the tables are the master): a new run keeps it, writes heads.csv and uses it."""
    db = tmp_path / "copy.duckdb"
    db.write_bytes(monaco.read_bytes())
    assert invoke(["levels", str(db)]).exit_code == 0
    import pandas as pd
    roads = set(pd.read_parquet(tmp_path / "copy.levels" / "roads.parquet")["road"])
    con = duckdb.connect(str(db))
    road = next(e for e, in con.execute("SELECT edge_id FROM driving.edges WHERE length_m > 50 ORDER BY edge_id").fetchall() if e in roads)   # a long road: the head fits
    con.execute(f"CREATE OR REPLACE TABLE visualization.level_heads AS SELECT * FROM (VALUES ({road}::BIGINT, '2.5', '')) t(road, start_m, end_m)")
    con.close()
    assert invoke(["levels", str(db)]).exit_code == 0
    assert str(road) in (tmp_path / "copy.levels" / "heads.csv").read_text()
    con = duckdb.connect(str(db), read_only=True)
    assert con.execute("SELECT head_start_m FROM visualization.edge_levels WHERE edge_id = ?", [road]).fetchone()[0] == 2.5


def test_levels_command_errors_say_what_is_wrong(tmp_path):
    db = tmp_path / "empty.duckdb"
    duckdb.connect(str(db)).close()
    r = invoke(["levels", str(db)])
    assert r.exit_code != 0 and "no <mode>.edges table" in r.output


def test_bus_routes_in_the_build_and_the_level_roads(monaco):
    """The build matches the bus route relations (bus.routes, bus.route_edges); the level roads get "bus" in modes and the lines in bus_lines.
    Boulevard Charles III (way 1449981121): #1f is the cars' one-way, #1r its contraflow bus lane."""
    con = duckdb.connect(str(monaco), read_only=True)
    assert con.execute("SELECT count(*) FROM bus.routes").fetchone()[0] > 0
    assert con.execute("SELECT count(*) FROM bus.route_edges WHERE bus_lane").fetchone()[0] > 0
    import pandas as pd
    roads = pd.read_parquet(monaco.with_suffix(".levels") / "roads.parquet")
    lines = {}                                           # edge_ref -> (modes of the edge, bus_lines of its road)
    for _, r in roads.iterrows():
        for side in ("edge", "reversed"):
            for ref, modes in zip(r["edge_refs" if side == "edge" else "reversed_refs"], r["edges_modes" if side == "edge" else "reversed_modes"]):
                lines[ref] = (modes, r["bus_lines"])
    modes, bus_lines = lines["1449981121#1r"]
    assert "bus" in modes.split(" + ") and "driving" not in modes.split(" + ")
    assert bus_lines == ", ".join(sorted(set(bus_lines.split(", ")), key=lambda r: (not r.isdigit(), int(r) if r.isdigit() else 0, r)))
    assert lines["1449981121#1f"][0].startswith("driving") and "bus" in lines["1449981121#1f"][0]
    assert roads["bus_lines"].isna().sum() > 0                                   # most roads have no bus
    for ms, bl in lines.values():                        # no line, no bus (2026-10-10: a walking-only reverse said "walking + bus")
        assert bl is not None or "bus" not in str(ms).split(" + ")


def test_the_build_writes_every_driving_edges_lane_profile(monaco):
    """driving.lane_profile (docs/design/gmns_lane_profile.md): every driving edge and every bus-only edge has its lanes, motor lanes
    numbered 1..n with no gap, a lane left of them -1, each with the source that put it there; Boulevard Charles III's contraflow bus lane
    shared by bikes, and the overridden Avenue Princesse Grace: its left bike lane and one car lane."""
    con = duckdb.connect(str(monaco), read_only=True)
    edges = {e for (e,) in con.execute("SELECT edge_id FROM driving.edges UNION ALL SELECT edge_id FROM driving.private_edges WHERE access = 'bus'").fetchall()}
    got = {}
    for e, n, u, s in con.execute("SELECT edge_id, lane_num, use, source FROM driving.lane_profile ORDER BY 1, 2").fetchall():
        got.setdefault(e, []).append((n, u, s))
    assert set(got) == edges and all(s for ls in got.values() for _, _, s in ls)
    for ls in got.values():
        motor = [n for n, _, s in ls if not s.startswith("cycleway")]          # the bike lanes beside them come from a cycleway tag
        assert motor == list(range(1, len(motor) + 1))
    ref = dict(con.execute("SELECT edge_ref, edge_id FROM driving.edges UNION ALL SELECT edge_ref, edge_id FROM driving.private_edges").fetchall())
    assert [u for _, u, _ in got[ref["1449981121#1r"]]] == ["bus,bike"]
    assert [(n, u) for n, u, _ in got[ref["503462464#2f"]]] == [(-1, "bike"), (1, "auto")]


def test_sumo_from_the_lane_profile_differs_from_sumos_own_import_only_by_named_causes(monaco, tmp_path):
    """sumo_compare.lane_diff (docs/design/gmns_lane_profile.md, decision 2): SUMO built from duckOSM's lane profile and SUMO's own OSM
    import of the same Monaco agree on most roads; every difference has a known cause (an override, the contraflow bus lane counted in
    lanes, a default where no lanes tag is ...), none is 'other'."""
    import shutil
    from duckosm.sumo import _find_netconvert
    try:
        _find_netconvert(None)
    except Exception:
        pytest.skip("netconvert is needed")
    if not shutil.which("osmium"):
        pytest.skip("osmium is needed")
    from duckosm.sumo_compare import lane_diff
    rows, n = lane_diff(monaco, ROOT / "data" / "sample" / "monaco.osm.pbf", tmp_path)
    assert n > 500 and len(rows) < 0.1 * n
    assert not [r for r in rows if r["cause"] == "other"], [r for r in rows if r["cause"] == "other"][:5]
