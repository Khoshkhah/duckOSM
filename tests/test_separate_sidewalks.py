"""SeparateSidewalks: a road with no sidewalk tag gets sidewalk=separate when both its sides are covered (a footway=sidewalk line,
or the other half of a dual carriageway); one side only, or a road with a sidewalk tag, is left as it is."""
import math

import duckdb

from duckosm.processors.separate_sidewalks import SeparateSidewalks

LAT = 59.0
M_LAT, M_LON = 1 / 111320.0, 1 / (111320.0 * math.cos(math.radians(LAT)))


def _db(ways):
    """``ways``: {osm_id: (tags, [(x_m, y_m), ...])} -> a connection with raw.nodes / raw.ways."""
    con = duckdb.connect()
    con.execute("CREATE SCHEMA raw")
    con.execute("CREATE TABLE raw.nodes (osm_id BIGINT, lat DOUBLE, lon DOUBLE, tags MAP(VARCHAR, VARCHAR))")
    con.execute("CREATE TABLE raw.ways (osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[])")
    nid = 0
    for wid, (tags, pts) in ways.items():
        refs = []
        for x, y in pts:
            nid += 1
            con.execute("INSERT INTO raw.nodes VALUES (?, ?, ?, MAP {})", [nid, LAT + y * M_LAT, 18.0 + x * M_LON])
            refs.append(nid)
        con.execute("INSERT INTO raw.ways VALUES (?, MAP(?, ?), ?)", [wid, list(tags), list(tags.values()), refs])
    return con


def test_a_road_with_sidewalk_lines_on_both_sides_or_the_other_carriageway_is_walked_on_them():
    sw = {"highway": "footway", "footway": "sidewalk"}
    con = _db({
        1: ({"highway": "residential"}, [(0, 0), (50, 0)]),                  # sidewalks both sides: tagged
        2: (sw, [(0, 5), (50, 5)]), 3: (sw, [(0, -5), (50, -5)]),
        4: ({"highway": "residential"}, [(0, 40), (50, 40)]),                # one side only: walked on the road
        5: (sw, [(0, 45), (50, 45)]),
        6: ({"highway": "residential", "sidewalk": "no"}, [(0, 80), (50, 80)]),   # tagged by OSM: left alone
        7: (sw, [(0, 85), (50, 85)]), 8: (sw, [(0, 75), (50, 75)]),
        # a dual carriageway: 9 east (other half on its left, north), 10 west; each has its sidewalk on its right (outside)
        9: ({"highway": "primary", "oneway": "yes"}, [(0, 120), (50, 120)]),
        10: ({"highway": "primary", "oneway": "yes"}, [(50, 128), (0, 128)]),
        11: (sw, [(0, 115), (50, 115)]), 12: (sw, [(0, 133), (50, 133)]),
        # a footway on a bridge over a road is no sidewalk of it
        13: ({"highway": "residential"}, [(0, 200), (50, 200)]),
        14: ({"highway": "footway", "footway": "sidewalk", "bridge": "yes"}, [(0, 204), (50, 204)]),
        15: (sw, [(0, 196), (50, 196)]),
    })
    assert SeparateSidewalks(con).run() == 3
    got = dict(con.execute("SELECT osm_id, map_extract(tags, 'duckosm:sidewalk')[1] FROM raw.ways WHERE map_extract(tags, 'sidewalk')[1] = 'separate'").fetchall())
    assert got == {1: "inferred", 9: "inferred", 10: "inferred"}
    assert con.execute("SELECT map_extract(tags, 'sidewalk')[1] FROM raw.ways WHERE osm_id = 6").fetchone()[0] == "no"
