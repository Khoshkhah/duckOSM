"""Each change that the fixes editor makes acts in `duckosm build` (docs/guides/edit-osm-data.md, "What each change writes").

tests/data/editor_changes/osm_fixes.yaml is the file that the editor wrote: one change of each kind that its forms make. The test builds
the Monaco sample with it in the two engines and checks the result of each change in the tables. The values before the change are in
the comments. The wall rule needs options.building_faces, which only the Rust engine has.
"""
from pathlib import Path

import duckdb
import pytest

from duckosm import Config, DuckOSM

ROOT = Path(__file__).resolve().parents[1]
FIXES = ROOT / "tests/data/editor_changes/osm_fixes.yaml"

# (what the editor changed, the SQL, the result with the file). Before the change: see the comment.
CHECKS = [
    ("road lanes", "SELECT DISTINCT lanes FROM driving.edges WHERE osm_id = 503462470", [(3,)]),                       # 1
    ("road name", "SELECT DISTINCT name FROM driving.edges WHERE osm_id = 503462470", [("Boulevard Test",)]),         # Boulevard Charles III
    ("road max speed", "SELECT DISTINCT maxspeed_kmh FROM driving.edges WHERE osm_id = 503462470", [(30.0,)]),        # 60
    ("road surface", "SELECT DISTINCT surface FROM driving.edges WHERE osm_id = 503462470", [("paving_stones",)]),   # asphalt
    ("road sidewalk", "SELECT DISTINCT walk_type FROM walking.edges WHERE osm_id = 503462470", [("sidewalk",)]),     # shared_road
    ("road turn lanes", "SELECT p.turn FROM driving.lane_profile p JOIN driving.edges e USING (edge_id) WHERE e.osm_id = 503462470 "
     "AND NOT e.is_reverse ORDER BY p.lane_num", [("left",), ("through",), ("through",)]),                            # no arrows
    ("road class", "SELECT DISTINCT highway FROM driving.edges WHERE osm_id = 157719645", [("service",)]),            # residential
    ("no cars", "SELECT (SELECT count(*) FROM driving.edges WHERE osm_id = 4230099), (SELECT count(*) > 0 FROM walking.edges WHERE osm_id = 4230099)",
     [(0, True)]),                                                                                                    # 18 driving edges
    ("no walking", "SELECT (SELECT count(*) FROM walking.edges WHERE osm_id = 4230011), (SELECT count(*) > 0 FROM driving.edges WHERE osm_id = 4230011)",
     [(0, True)]),                                                                                                    # 14 walking edges
    ("reverse one-way", "SELECT count(*), bool_and(oneway) FROM driving.edges WHERE osm_id = 165636031", [(6, True)]),   # 12, two-way
    ("layer and bridge", "SELECT DISTINCT layer, bridge FROM driving.edges WHERE osm_id = 766768153", [("1", "yes")]),   # none
    ("delete a road", "SELECT (SELECT count(*) FROM driving.edges WHERE osm_id = 4229577) + (SELECT count(*) FROM walking.edges WHERE osm_id = 4229577)",
     [(0,)]),                                                                                                         # 12
    ("building tags", "SELECT tags['building:levels'], tags['height'], tags['roof:shape'], tags['building:colour'] FROM features.buildings "
     "WHERE osm_id = 23715051", [("9", "30", "flat", "#ffffff")]),                                                    # 2 levels, no others
    ("delete a building way", "SELECT count(*) FROM features.buildings WHERE osm_id = 1152567278", [(0,)]),          # 1
    ("delete a building relation", "SELECT count(*) FROM features.buildings WHERE osm_id = 393226 AND osm_type = 'relation'", [(0,)]),   # 1
    ("new traffic signal", "SELECT kind FROM features.traffic WHERE osm_id = 21912089", [("traffic_signals",)]),     # not in the table
    ("delete a traffic signal", "SELECT count(*) FROM features.traffic WHERE osm_id = 13135657937", [(0,)]),         # 1
    ("new crossing", "SELECT kind, tags['crossing'], tags['kerb'] FROM features.traffic WHERE osm_id = 21912099", [("crossing", "marked", "lowered")]),
    ("turn rule only", "SELECT restriction_type FROM driving.turn_restrictions WHERE via_node = 21912099", [("only_right_turn",)]),   # none
    ("turn rule no", "SELECT restriction_type FROM driving.turn_restrictions WHERE via_node = 21912089", [("no_straight_on",)]),      # none
    ("new road", "SELECT count(*), any_value(name), any_value(highway) FROM driving.edges WHERE osm_id = 9000000000000", [(2, "New lane", "service")]),
]
WALL = ("wall type and door", "SELECT face_type, computed_type, edited, door FROM main.building_faces WHERE building_id = 23715051 "
        "AND from_node = 256791677 AND to_node = 256791676", [("front", "side", True, "main")])                   # side, no door


def _build(out, engine):
    cfg = Config.from_args(str(ROOT / "data/sample/monaco.osm.pbf"), str(out), name="monaco", boundary_path=str(ROOT / "data/sample/monaco.geojson"),
                           modes=["driving", "walking", "cycling"], osm_overrides=str(FIXES), building_faces=engine == "rust")
    cfg.options.engine = engine
    cfg.options.sea = None                     # the sea comes from the network (S3): not in a test
    cfg.report.enabled = False
    con = duckdb.connect(str(DuckOSM(cfg).run()), read_only=True)
    con.execute("LOAD spatial")
    return con


@pytest.mark.parametrize("engine", ["rust", "python"])
def test_each_change_of_the_editor_acts_in_the_build(tmp_path, engine):
    con = _build(tmp_path, engine)
    checks = CHECKS + ([WALL] if engine == "rust" else [])
    bad = [f"{name}: {got} != {want}" for name, sql, want in checks if (got := con.execute(sql).fetchall()) != want]
    assert not bad, "\n".join(bad)


def test_wall_rules_without_the_walls_option_give_a_warning(tmp_path, caplog):
    cfg = Config.from_args(str(ROOT / "data/sample/monaco.osm.pbf"), str(tmp_path), name="monaco", boundary_path=str(ROOT / "data/sample/monaco.geojson"),
                           modes=["driving"], osm_overrides=str(FIXES))
    cfg.options.sea = None
    cfg.report.enabled = False
    with caplog.at_level("WARNING", logger="duckosm"):
        DuckOSM(cfg).run()
    assert any("1 wall rule(s)" in r.getMessage() and "options.building_faces" in r.getMessage() for r in caplog.records)
