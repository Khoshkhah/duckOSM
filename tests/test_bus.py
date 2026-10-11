"""Bus routes: an OSM bus route relation as the ordered driving edges it travels, in its direction (src/duckosm/bus.py)."""
import duckdb

from duckosm.bus import match_bus_routes

WAYS = {  # osm_id: (nodes, tags)
    10: ([1, 2, 3], {"highway": "primary"}),                                  # drawn along the route 1 -> 3, two edges
    20: ([5, 4, 3], {"highway": "primary"}),                                  # drawn against it: the route goes 3 -> 5
    30: ([6, 5], {"highway": "primary", "oneway": "yes", "oneway:bus": "no"}),  # the route goes 5 -> 6: the bus lane
    40: ([9, 6, 7], {"highway": "primary"}),                                  # entered in the middle: the role says which way
    50: ([100, 101], {"highway": "footway", "oneway": "yes"}),                # not a road for cars
    70: ([1, 11], {"highway": "service"}),                                    # a private road
    80: ([20, 21, 22, 23, 20], {"highway": "primary", "junction": "roundabout"}),
    81: ([30, 20], {"highway": "primary"}),
    82: ([22, 40], {"highway": "primary"}),
    99: ([200, 201], {"public_transport": "platform"}),
}
EDGES = [  # (edge_ref, nodes); private ones with their access
    ("10#1f", [1, 2]), ("10#2f", [2, 3]), ("10#1r", [2, 1]), ("10#2r", [3, 2]),
    ("20#1f", [5, 4]), ("20#2f", [4, 3]), ("20#1r", [4, 5]), ("20#2r", [3, 4]),
    ("30#1f", [6, 5]),
    ("40#1f", [9, 6]), ("40#2f", [6, 7]), ("40#1r", [6, 9]), ("40#2r", [7, 6]),
    ("80#1f", [20, 21]), ("80#2f", [21, 22, 23, 20]),
    ("81#1f", [30, 20]), ("82#1f", [22, 40]),
]
PRIVATE = [("30#1r", [5, 6], "bus"), ("70#1f", [1, 11], "private")]
RELATIONS = {  # osm_id: (ref, members [(type, ref, role)])
    1: ("1", [("node", 500, "stop"), ("way", 10, ""), ("way", 20, ""), ("way", 30, ""), ("way", 40, "backward"), ("way", 99, "platform")]),
    2: ("2", [("way", 10, "forward"), ("way", 50, ""), ("way", 999, "")]),
    3: ("3", [("way", 30, ""), ("way", 20, ""), ("way", 10, "forward"), ("way", 70, "")]),
    4: ("4", [("way", 81, ""), ("way", 80, ""), ("way", 82, "")]),
}


def _db():
    con = duckdb.connect()
    con.execute("CREATE SCHEMA raw; CREATE SCHEMA driving")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[])")
    con.execute("CREATE TABLE raw.relations(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[], ref_roles VARCHAR[], ref_types VARCHAR[])")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, edge_ref VARCHAR, osm_id BIGINT, refs BIGINT[])")
    con.execute("CREATE TABLE driving.private_edges(edge_id BIGINT, edge_ref VARCHAR, osm_id BIGINT, refs BIGINT[], access VARCHAR)")
    for w, (nodes, tags) in WAYS.items():
        con.execute("INSERT INTO raw.ways VALUES (?, MAP(?, ?), ?)", [w, list(tags), list(tags.values()), nodes])
    for i, (ref, nodes) in enumerate(EDGES):
        con.execute("INSERT INTO driving.edges VALUES (?, ?, ?, ?)", [i, ref, int(ref.split("#")[0]), nodes])
    for i, (ref, nodes, access) in enumerate(PRIVATE, start=100):
        con.execute("INSERT INTO driving.private_edges VALUES (?, ?, ?, ?, ?)", [i, ref, int(ref.split("#")[0]), nodes, access])
    for r, (ref, members) in RELATIONS.items():
        t, m, role = zip(*members, strict=True)
        con.execute("INSERT INTO raw.relations VALUES (?, MAP(['type', 'route', 'ref'], ['route', 'bus', ?]), ?, ?, ?)", [r, ref, list(m), list(role), list(t)])
    return con


def _route(df, r):
    return list(df[df.relation_id == r].sort_values("seq").edge_ref)


def _gaps(df, r):
    g = df.attrs["gaps"]
    return list(g[g.relation_id == r][["way_id", "kind"]].itertuples(index=False, name=None))


def test_directions_from_the_neighbouring_ways_and_the_roles():
    """Split ways in both drawing directions, a contraflow bus lane, a way entered in its middle (role backward); stops and platforms skipped."""
    df = match_bus_routes(_db())
    assert _route(df, 1) == ["10#1f", "10#2f", "20#2r", "20#1r", "30#1r", "40#1r"]
    assert list(df[df.relation_id == 1].bus_lane) == [False] * 4 + [True, False]
    assert list(df[df.relation_id == 1].seq) == [1, 2, 3, 4, 5, 6]
    assert _gaps(df, 1) == []


def test_gaps_are_reported():
    """A way not joined to the next, a way with no car edge, a way outside the file, a private road, a role the ways contradict."""
    df = match_bus_routes(_db())
    assert _route(df, 2) == ["10#1f", "10#2f"]
    assert _gaps(df, 2) == [(10, "no_connection"), (50, "no_edge"), (999, "outside")]
    assert _route(df, 3) == ["30#1f", "20#1f", "20#2f", "10#2r", "10#1r"]       # 10 travelled backwards: its role forward is wrong
    assert _gaps(df, 3) == [(10, "role_conflict"), (70, "private")]


def test_a_roundabout_from_entry_to_exit():
    df = match_bus_routes(_db())
    assert _route(df, 4) == ["81#1f", "80#1f", "80#2f", "82#1f"]
    assert _gaps(df, 4) == []


def test_the_bus_tables():
    """bus.routes: one row per route with its gaps (outside the area counted apart); bus.route_edges: the edges in order."""
    from duckosm.bus import write_bus_routes
    con = _db()
    write_bus_routes(con)
    rows = {r[0]: r[1:] for r in con.execute("SELECT osm_id, ref, edges, outside, gaps FROM bus.routes").fetchall()}
    assert rows == {1: ("1", 6, 0, []), 2: ("2", 2, 1, ["10 no_connection", "50 no_edge"]),
                    3: ("3", 5, 0, ["10 role_conflict", "70 private"]), 4: ("4", 4, 0, [])}
    assert con.execute("SELECT list(edge_ref ORDER BY seq) FROM bus.route_edges WHERE route_id = 1").fetchone()[0] == \
        ["10#1f", "10#2f", "20#2r", "20#1r", "30#1r", "40#1r"]
    assert con.execute("SELECT count(*) FROM bus.route_edges WHERE bus_lane").fetchone()[0] == 1


def test_bus_command_writes_the_tables(tmp_path):
    from click.testing import CliRunner

    from duckosm.cli import main
    db = tmp_path / "routes.duckdb"         # not bus.duckdb: its catalog name would clash with the schema bus
    con = _db()
    con.execute(f"ATTACH '{db}' AS f; COPY FROM DATABASE memory TO f; DETACH f")
    con.close()
    r = CliRunner().invoke(main, ["bus", str(db)])
    assert r.exit_code == 0, r.output
    assert "4 bus routes, 17 route edges, 2 with gaps" in r.output
    empty = tmp_path / "empty.duckdb"
    duckdb.connect(str(empty)).close()
    r = CliRunner().invoke(main, ["bus", str(empty)])
    assert r.exit_code != 0 and "needs raw.relations" in r.output
