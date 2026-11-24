"""Separate sidewalks: a road with no sidewalk tag gets sidewalk=separate (and duckosm:sidewalk=inferred) when both its
sides are covered (a footway=sidewalk line, or the other half of a dual carriageway); one side only, or a road with a
sidewalk tag, is left as it is. Checked on the tags and the walk_type of the walking network of the Rust build."""
import math

import duckdb

from osm_build import build

LAT = 43.0
M_LAT, M_LON = 1 / 111320.0, 1 / (111320.0 * math.cos(math.radians(LAT)))


def _build(tmp_path, ways, args=()):
    """``ways``: {osm_id: (tags, [(x_m, y_m), ...])} -> read-only connection to the walking build."""
    nodes, wd, nid = {}, {}, 0
    for wid, (tags, pts) in ways.items():
        refs = []
        for x, y in pts:
            nid += 1
            nodes[nid] = (7.0 + x * M_LON, LAT + y * M_LAT)
            refs.append(nid)
        wd[wid] = (refs, tags)
    return duckdb.connect(str(build(tmp_path, nodes, wd, modes=("walking",), args=args)), read_only=True)


SW = {"highway": "footway", "footway": "sidewalk"}
WAYS = {
    1: ({"highway": "residential"}, [(0, 0), (50, 0)]),                  # sidewalks both sides: tagged
    2: (SW, [(0, 5), (50, 5)]), 3: (SW, [(0, -5), (50, -5)]),
    4: ({"highway": "residential"}, [(0, 40), (50, 40)]),                # one side only: walked on the road
    5: (SW, [(0, 45), (50, 45)]),
    6: ({"highway": "residential", "sidewalk": "no"}, [(0, 80), (50, 80)]),   # tagged by OSM: left alone
    7: (SW, [(0, 85), (50, 85)]), 8: (SW, [(0, 75), (50, 75)]),
    # a dual carriageway: 9 east (other half on its left, north), 10 west; each has its sidewalk on its right (outside)
    9: ({"highway": "primary", "oneway": "yes"}, [(0, 120), (50, 120)]),
    10: ({"highway": "primary", "oneway": "yes"}, [(50, 128), (0, 128)]),
    11: (SW, [(0, 115), (50, 115)]), 12: (SW, [(0, 133), (50, 133)]),
    # a footway on a bridge over a road is no sidewalk of it
    13: ({"highway": "residential"}, [(0, 200), (50, 200)]),
    14: ({"highway": "footway", "footway": "sidewalk", "bridge": "yes"}, [(0, 204), (50, 204)]),
    15: (SW, [(0, 196), (50, 196)]),
}


def test_a_road_with_sidewalk_lines_on_both_sides_or_the_other_carriageway_is_walked_on_them(tmp_path):
    con = _build(tmp_path, WAYS)
    # A road whose sidewalk is a separate way is not walked: the people walk the sidewalk line.
    walked = {r[0] for r in con.execute("SELECT osm_id FROM walking.ways").fetchall()}
    assert {1, 9, 10}.isdisjoint(walked)
    assert {2, 3, 5, 11, 12, 15}.issubset(walked)
    # One side only, tagged by OSM, or a bridge footway: left as it is, walked on the road.
    assert {4, 6, 13}.issubset(walked)
    assert con.execute("SELECT walk_type FROM walking.edges WHERE osm_id = 4 LIMIT 1").fetchone()[0] == "shared_road"


def test_the_rule_is_off_with_infer_separate_sidewalks_false(tmp_path):
    cfg = tmp_path / "off.yaml"
    cfg.write_text("options:\n  infer_separate_sidewalks: false\n")
    con = _build(tmp_path, WAYS, args=["-c", cfg])
    walked = {r[0] for r in con.execute("SELECT osm_id FROM walking.ways").fetchall()}
    assert {1, 9, 10} <= walked                    # no inference: the roads are walked
