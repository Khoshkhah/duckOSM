"""The merge of same-road chains (option merge_segments, on by default) in the Rust build, and the table <mode>.edge_id_map.

With the default ``global_junctions: true`` a node where two road ways meet is always a junction, so the build does not merge there
(rust/duckosm-core/tests/modes.rs says the same of Monaco). These tests turn it off (``_build``), the case where the merge works.
Each test writes the OSM ways of a small network (tests/osm_build.py), builds it and reads the tables of the db: ``driving.edges``,
``driving.nodes``, ``driving.edge_id_map``, ``driving.turn_restrictions`` and ``driving.edge_graph``.
"""
import duckdb
import pytest

from duckosm import Config
from osm_build import build as _osm_build

X = 0.001   # 0.001 degrees of longitude at lat 43 is about 81 m


def build(tmp_path, nodes, ways, relations=None):
    """The Rust build with ``global_junctions: false`` (the merge only runs then)."""
    cfg = tmp_path / "gj_off.yaml"
    cfg.write_text("name: t\noptions: {global_junctions: false}\n")
    return _osm_build(tmp_path, nodes, ways, relations, args=["-c", cfg])


def _con(db):
    con = duckdb.connect(str(db), read_only=True)
    con.execute("LOAD spatial")
    return con


def _h(con, o, s, t):
    """The stable edge id of (osm_id, source, target)."""
    return con.execute("SELECT (hash(?::BIGINT, ?::BIGINT, ?::BIGINT) >> 1)::BIGINT", [o, s, t]).fetchone()[0]


def _road(tags=None):
    return {"highway": "residential", "name": "Foo", "maxspeed": "50", "lanes": "2", **(tags or {})}


# A: a two-way chain split across two ways (100, 200) with equal tags: it merges. B: the same, but the maxspeed differs: it stays split.
# C: a closed loop (way 400) that has a stick: its self-loop is cut at a virtual node and the halves stay separate.
def _graph(tmp_path):
    nodes = {1: (7.0, 43.0), 2: (7.0 + X, 43.0), 3: (7.0 + 3 * X, 43.0),
             20: (7.1, 43.0), 21: (7.1 + X, 43.0), 22: (7.1 + 2 * X, 43.0),
             30: (7.2, 43.0), 31: (7.2 + X, 43.0), 32: (7.2 + 1.5 * X, 43.0 + X), 33: (7.2 + 1.5 * X, 43.0 - X)}
    ways = {100: ([1, 2], _road()), 200: ([2, 3], _road()),
            300: ([20, 21], _road({"maxspeed": "50", "oneway": "yes", "lanes": "1"})),
            301: ([21, 22], _road({"maxspeed": "30", "oneway": "yes", "lanes": "1"})),
            400: ([30, 31, 32, 33, 31], _road({"lanes": "1"}))}
    return build(tmp_path, nodes, ways)


def test_merge_segments_config(tmp_path):
    base = "name: t\nsource: {type: pbf, pbf_path: x}\n"
    d = tmp_path / "d.yaml"
    d.write_text(base + "")
    assert Config.from_yaml(str(d)).options.merge_segments is True           # default on
    e = tmp_path / "e.yaml"
    e.write_text(base + "options: {merge_segments: false}\n")
    assert Config.from_yaml(str(e)).options.merge_segments is False          # opt out


def test_chain_of_two_ways_merges_into_one_edge_pair(tmp_path):
    con = _con(_graph(tmp_path))
    rows = con.execute("SELECT source, target, osm_id, refs, length_m, ST_NPoints(geometry), is_reverse FROM driving.edges "
                       "WHERE osm_id IN (100, 200) ORDER BY is_reverse").fetchall()
    assert len(rows) == 2                                     # one forward edge, one reverse twin
    s, t, osm, refs, length, npts, _ = rows[0]
    assert (s, t) == (1, 3) and (rows[1][0], rows[1][1]) == (3, 1)
    assert osm == 100                                         # the first (source-end) way is the representative
    assert refs == [1, 2, 3] and npts == 3                    # stitched through the middle node, not repeated
    assert length == pytest.approx(3 * X * 111195 * 0.7314, rel=0.02)   # the lengths of the members, summed
    assert con.execute("SELECT count(*) FROM driving.nodes WHERE node_id = 2").fetchone()[0] == 0   # node 2 is not a node any more


def test_two_way_chain_with_both_ways_drawn_out_of_the_shared_node_merges(tmp_path):
    """A two-way chain whose ways both start at the shared node 2 (2 -> 1 and 2 -> 3) must still merge."""
    nodes = {1: (7.0, 43.0), 2: (7.0 + X, 43.0), 3: (7.0 + 2 * X, 43.0)}
    db = build(tmp_path, nodes, {700: ([2, 1], _road()), 701: ([2, 3], _road())})
    rows = _con(db).execute("SELECT source, target, refs FROM driving.edges WHERE NOT is_reverse").fetchall()
    assert len(rows) == 1
    s, t, refs = rows[0]
    assert sorted([s, t]) == [1, 3] and sorted(refs) == [1, 2, 3]


def test_a_changed_attribute_or_a_cut_self_loop_stops_the_merge(tmp_path):
    con = _con(_graph(tmp_path))
    # maxspeed 50 then 30 at node 21: both segments survive
    assert con.execute("SELECT count(*) FROM driving.edges WHERE osm_id IN (300, 301)").fetchone()[0] == 2
    # the self-loop of way 400 is cut at a virtual node (id < 0); its halves are not merged back
    assert con.execute("SELECT count(*) FROM driving.nodes WHERE node_id < 0").fetchone()[0] >= 1
    assert con.execute("SELECT count(*) FROM driving.edges WHERE osm_id = 400 AND NOT is_reverse").fetchone()[0] >= 2
    assert con.execute("SELECT count(*) FROM driving.edges WHERE source = target").fetchone()[0] == 0


def test_a_one_way_chain_keeps_its_legal_direction(tmp_path):
    """The ways run 52 -> 51 -> 50 (legal direction); the merged edge must run 52 -> 50, not from the lower node id."""
    nodes = {52: (7.0, 43.0), 51: (7.0 + X, 43.0), 50: (7.0 + 2 * X, 43.0)}
    db = build(tmp_path, nodes, {500: ([52, 51], _road({"oneway": "yes"})), 501: ([51, 50], _road({"oneway": "yes"}))})
    rows = _con(db).execute("SELECT source, target, osm_id, refs FROM driving.edges").fetchall()
    assert rows == [(52, 50, 500, [52, 51, 50])]             # one edge, no reverse twin


def test_merged_edge_geometry_starts_on_its_source_and_ends_on_its_target(tmp_path):
    con = _con(_graph(tmp_path))
    bad = con.execute("""
        SELECT e.edge_id FROM driving.edges e
        JOIN driving.nodes s ON s.node_id = e.source JOIN driving.nodes t ON t.node_id = e.target
        WHERE ST_Distance(ST_StartPoint(e.geometry), s.geom) > 1e-7 OR ST_Distance(ST_EndPoint(e.geometry), t.geom) > 1e-7""").fetchall()
    assert bad == []


def test_a_self_loop_is_cut_at_a_virtual_node_on_the_geometry(tmp_path):
    """The virtual node of a split self-loop sits where the two halves are cut (half of the length), not at a middle vertex."""
    # a loop 1-2-3-4-1 with unevenly spaced vertices: the middle vertex is not the half-length point
    nodes = {1: (7.0, 43.0), 2: (7.0 + 3 * X, 43.0), 3: (7.0 + 3 * X, 43.0 + X), 4: (7.0, 43.0 + X)}
    con = _con(build(tmp_path, nodes, {700: ([1, 2, 3, 4, 1], _road({"lanes": "1"}))}))
    assert con.execute("SELECT count(*) FROM driving.edges WHERE source = target").fetchone()[0] == 0
    v = con.execute("SELECT node_id, geom FROM driving.nodes WHERE node_id < 0").fetchall()
    assert v
    bad = con.execute("""
        SELECT e.edge_id FROM driving.edges e JOIN driving.nodes n ON n.node_id < 0 AND n.node_id IN (e.source, e.target)
        WHERE (e.target = n.node_id AND ST_Distance(ST_EndPoint(e.geometry), n.geom) > 1e-7)
           OR (e.source = n.node_id AND ST_Distance(ST_StartPoint(e.geometry), n.geom) > 1e-7)""").fetchall()
    assert bad == []
    # the loop is cut at half of its length; the second half is cut again (the two pieces point against each other)
    lens = [r[0] for r in con.execute("SELECT length_m FROM driving.edges WHERE NOT is_reverse ORDER BY edge_ref").fetchall()]
    assert lens[0] == pytest.approx(sum(lens) / 2, rel=0.01)


def test_edge_id_map_lists_the_old_ids_of_the_merged_edges(tmp_path):
    con = _con(_graph(tmp_path))
    rows = con.execute("SELECT old_edge_id, new_edge_id, seq, is_reverse, osm_id FROM driving.edge_id_map "
                       "WHERE new_edge_id IN (SELECT edge_id FROM driving.edges WHERE osm_id = 100)").fetchall()
    assert len({r[1] for r in rows}) == 2 and len({r[0] for r in rows}) == 4     # 2 forward + 2 reverse members -> 1 + 1 merged edges
    assert {r[2] for r in rows} == {1, 2}
    # the new ids are the stable hash of the merged edge (osm_id 100, 1 -> 3 and 3 -> 1)
    assert {r[1] for r in rows} == {_h(con, 100, 1, 3), _h(con, 100, 3, 1)}
    # the first forward piece keeps the id it had without the merge; the piece list gives its way
    assert (_h(con, 100, 1, 2), _h(con, 100, 1, 3), 1, False, 100) in rows
    assert (_h(con, 200, 2, 3), _h(con, 100, 1, 3), 2, False, 200) in rows


def test_edge_id_map_pieces_point_the_same_way_as_the_merged_edge(tmp_path):
    """Way 200 is drawn 3 -> 2, against the chain 1 -> 3. Its old id must map to the merged edge that runs the same way (3 -> 1)."""
    nodes = {1: (7.0, 43.0), 2: (7.0 + X, 43.0), 3: (7.0 + 3 * X, 43.0)}
    con = _con(build(tmp_path, nodes, {100: ([1, 2], _road()), 200: ([3, 2], _road())}))
    m = {(r[0], r[1]): r[2] for r in con.execute("SELECT old_edge_id, new_edge_id, is_reverse FROM driving.edge_id_map").fetchall()}
    fwd, rev = _h(con, 100, 1, 3), _h(con, 100, 3, 1)
    assert m == {(_h(con, 100, 1, 2), fwd): False, (_h(con, 200, 2, 3), fwd): True,
                 (_h(con, 100, 2, 1), rev): True, (_h(con, 200, 3, 2), rev): False}


def test_edge_id_map_is_empty_when_nothing_merges(tmp_path):
    nodes = {1: (7.0, 43.0), 2: (7.0 + X, 43.0), 3: (7.0 + 2 * X, 43.0)}
    ways = {10: ([1, 2], _road({"maxspeed": "50"})), 11: ([2, 3], _road({"maxspeed": "30"}))}
    assert _con(build(tmp_path, nodes, ways)).execute("SELECT count(*) FROM driving.edge_id_map").fetchone()[0] == 0


def test_restriction_matches_the_incident_end_way_of_a_merged_edge(tmp_path):
    """The from-way 502 is the target-end member of the merged edge 500 + 502; the restriction must still find that edge."""
    nodes = {1: (7.0, 43.0), 2: (7.0 + X, 43.0), 10: (7.0 + 2 * X, 43.0), 30: (7.0 + 3 * X, 43.0), 40: (7.0 + 2 * X, 43.0 + X)}
    ways = {500: ([1, 2], _road()), 502: ([2, 10], _road()), 600: ([10, 30], _road({"name": "Bar"})), 601: ([10, 40], _road({"name": "Baz"}))}
    rel = {999: ([("way", 502, "from"), ("node", 10, "via"), ("way", 600, "to")], {"type": "restriction", "restriction": "no_left_turn"})}
    con = _con(build(tmp_path, nodes, ways, rel))
    rows = con.execute("SELECT restriction_type, via_node, from_edge_id, to_edge_id FROM driving.turn_restrictions").fetchall()
    assert rows == [("no_left_turn", 10, _h(con, 500, 1, 10), _h(con, 600, 10, 30))]   # from-edge is the merge of 500 and 502


def test_edge_graph_enforces_no_and_only_restrictions(tmp_path):
    """A no_* restriction drops the named turn; an only_* restriction drops every other turn from the from-edge."""
    nodes = {10: (7.0, 43.0), 1: (7.0 - X, 43.0), 5: (7.0, 43.0 - X), 2: (7.0 + X, 43.0), 3: (7.0, 43.0 + X), 4: (7.0 - X, 43.0 + X)}
    ways = {w: ([a, 10] if a in (1, 5) else [10, a], _road({"oneway": "yes", "name": f"R{w}"})) for w, a in ((1, 1), (5, 5), (2, 2), (3, 3), (4, 4))}
    rel = {1: ([("way", 1, "from"), ("node", 10, "via"), ("way", 2, "to")], {"type": "restriction", "restriction": "only_straight_on"}),
           2: ([("way", 5, "from"), ("node", 10, "via"), ("way", 3, "to")], {"type": "restriction", "restriction": "no_right_turn"})}
    con = _con(build(tmp_path, nodes, ways, rel))
    e = {w: _h(con, w, a, 10) if a in (1, 5) else _h(con, w, 10, a) for w, a in ((1, 1), (5, 5), (2, 2), (3, 3), (4, 4))}
    succ = lambda f: sorted(r[0] for r in con.execute("SELECT to_edge FROM driving.edge_graph WHERE from_edge = ?", [e[f]]).fetchall())
    assert succ(1) == [e[2]]                                     # only_*: just the mandated turn
    assert succ(5) == sorted([e[2], e[4]])                       # no_*: only the prohibited turn (3) is dropped
