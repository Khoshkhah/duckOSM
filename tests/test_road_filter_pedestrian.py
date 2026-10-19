"""Driving keeps every highway=pedestrian way. One cars may use becomes living_street (in edges); the rest
get a label in `access` and move to private_edges (docs/design/pedestrian_private_edges.md)."""
from types import SimpleNamespace

import duckdb
import pytest

from duckosm.importer import DuckOSM
from duckosm.processors.road_filter import RoadFilter


def _way(mode, tags):
    con = duckdb.connect()
    con.execute("CREATE SCHEMA raw")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[])")
    kv = ", ".join(f"'{k}': '{v}'" for k, v in tags.items())
    con.execute(f"INSERT INTO raw.ways VALUES (1, MAP {{{kv}}}, [1, 2])")
    RoadFilter(con, mode=mode)._create_ways_table()
    return con, con.execute("SELECT highway, access FROM ways").fetchall()


@pytest.mark.parametrize("tags, expected", [
    ({"highway": "pedestrian"}, ("pedestrian", "pedestrian")),
    ({"highway": "pedestrian", "motor_vehicle": "no"}, ("pedestrian", "pedestrian")),
    ({"highway": "pedestrian", "access": "no"}, ("pedestrian", "pedestrian")),
    ({"highway": "pedestrian", "vehicle": "yes"}, ("pedestrian", "pedestrian")),
    ({"highway": "pedestrian", "motor_vehicle": "private"}, ("pedestrian", "private")),
    ({"highway": "pedestrian", "access": "private"}, ("pedestrian", "private")),
    ({"highway": "pedestrian", "motor_vehicle": "delivery"}, ("pedestrian", "delivery")),
    # cars may use it: rescued as living_street, as before
    ({"highway": "pedestrian", "motor_vehicle": "yes"}, ("living_street", "yes")),
    ({"highway": "pedestrian", "access": "delivery"}, ("living_street", "delivery")),
    ({"highway": "pedestrian", "access": "destination"}, ("living_street", "destination")),
])
def test_driving_pedestrian_label(tags, expected):
    _, rows = _way("driving", tags)
    assert rows == [expected]


def test_other_modes_unchanged():
    _, rows = _way("walking", {"highway": "pedestrian"})
    assert rows[0][0] == "pedestrian"


def test_split_private_moves_only_driving_pedestrian_streets():
    con = duckdb.connect()
    con.execute("CREATE TABLE nodes(node_id BIGINT)")
    con.execute("INSERT INTO nodes VALUES (1), (2), (3), (4)")
    con.execute("CREATE TABLE edges(edge_id BIGINT, source BIGINT, target BIGINT, highway VARCHAR, access VARCHAR)")
    con.execute("""INSERT INTO edges VALUES (10, 1, 2, 'tertiary', NULL), (11, 2, 3, 'pedestrian', 'pedestrian'),
                   (12, 3, 4, 'unclassified', 'bus'), (13, 1, 4, 'living_street', 'yes')""")
    DuckOSM._split_private(SimpleNamespace(con=con), "driving")
    assert con.execute("SELECT edge_id FROM edges ORDER BY 1").fetchall() == [(10,), (13,)]
    assert con.execute("SELECT edge_id FROM private_edges ORDER BY 1").fetchall() == [(11,), (12,)]
    # walking keeps its pedestrian streets
    con.execute("DROP TABLE private_edges")
    con.execute("INSERT INTO edges VALUES (14, 1, 2, 'pedestrian', NULL)")
    DuckOSM._split_private(SimpleNamespace(con=con), "walking")
    assert (14,) in con.execute("SELECT edge_id FROM edges").fetchall()
