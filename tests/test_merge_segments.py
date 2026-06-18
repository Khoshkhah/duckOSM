"""
Tests for options.merge_segments — GraphSimplifier._contract_chains() (v2: merge same-road
degree-2 chains across osm_id) and the <mode>.edge_id_map matching table. Run: pytest tests/ -q
"""
import duckdb
import pytest

from duckosm import Config
from duckosm.processors.graph_simplifier import GraphSimplifier


def _graph():
    """A synthetic forward-edge graph with: one mergeable same-road chain split across two
    osm_ids, a pair that differs only in maxspeed (must NOT merge), and a self-loop split at
    a virtual midpoint with a negative node id (must NOT merge)."""
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA raw")
    con.execute("CREATE TABLE raw.nodes(osm_id BIGINT, lon DOUBLE, lat DOUBLE)")
    con.execute("INSERT INTO raw.nodes VALUES (1,0,0),(2,1,0),(3,3,0)")
    con.execute("""CREATE TABLE simplified_edges_forward(
        edge_id INTEGER, source BIGINT, target BIGINT, osm_id BIGINT, highway VARCHAR,
        name VARCHAR, maxspeed VARCHAR, oneway BOOLEAN, lanes INTEGER, surface VARCHAR,
        junction VARCHAR, refs BIGINT[], geometry GEOMETRY, is_reverse BOOLEAN, length_m FLOAT)""")
    g = lambda w: f"ST_GeomFromText('{w}')"
    con.execute(f"""INSERT INTO simplified_edges_forward VALUES
      -- same road, different osm_id, all attrs equal -> MERGE (first/source-end -> osm_id 100 wins)
      (1,1,2,100,'residential','Foo','50',false,2,NULL,NULL,[1,2],{g('LINESTRING(0 0,1 0)')},false,111195),
      (2,2,3,200,'residential','Foo','50',false,2,NULL,NULL,[2,3],{g('LINESTRING(1 0,3 0)')},false,222390),
      -- differs only in maxspeed at node 21 -> NOT merged
      (3,20,21,300,'tertiary','Bar','50',true,1,NULL,NULL,[20,21],{g('LINESTRING(7 0,8 0)')},true,111195),
      (4,21,22,301,'tertiary','Bar','30',true,1,NULL,NULL,[21,22],{g('LINESTRING(8 0,9 0)')},true,111195),
      -- virtual self-loop midpoint (-999) -> NOT merged (node id <= 0)
      (5,30,-999,400,'residential','Loop','50',false,1,NULL,NULL,[30,-999],{g('LINESTRING(1 1,1 2)')},false,50),
      (6,-999,31,400,'residential','Loop','50',false,1,NULL,NULL,[-999,31],{g('LINESTRING(1 2,1 3)')},false,50)""")
    return con


def _oneway_graph():
    """A one-way chain split across two osm_ids whose LEGAL travel direction runs from the
    higher-id terminal to the lower (52 -> 51 -> 50). The old `start < cur` orientation would
    have stored it reversed (50 -> 52); the merge must keep the legal direction so the single
    forward edge is drivable the right way. Geometry runs west->east (x: 10 -> 11 -> 12)."""
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA raw")
    con.execute("CREATE TABLE raw.nodes(osm_id BIGINT, lon DOUBLE, lat DOUBLE)")
    con.execute("INSERT INTO raw.nodes VALUES (50,12,0),(51,11,0),(52,10,0)")
    con.execute("""CREATE TABLE simplified_edges_forward(
        edge_id INTEGER, source BIGINT, target BIGINT, osm_id BIGINT, highway VARCHAR,
        name VARCHAR, maxspeed VARCHAR, oneway BOOLEAN, lanes INTEGER, surface VARCHAR,
        junction VARCHAR, refs BIGINT[], geometry GEOMETRY, is_reverse BOOLEAN, length_m FLOAT)""")
    g = lambda w: f"ST_GeomFromText('{w}')"
    con.execute(f"""INSERT INTO simplified_edges_forward VALUES
      (1,52,51,500,'residential','One','50',true,1,NULL,NULL,[52,51],{g('LINESTRING(10 0,11 0)')},false,111195),
      (2,51,50,501,'residential','One','50',true,1,NULL,NULL,[51,50],{g('LINESTRING(11 0,12 0)')},false,111195)""")
    return con


# ---- config flag -------------------------------------------------------------------
def test_merge_segments_config(tmp_path):
    base = "name: t\nsource: {type: pbf, pbf_path: x}\n"
    d = tmp_path / "d.yaml"
    d.write_text(base + "options: {simplify: true}\n")
    assert Config.from_yaml(str(d)).options.merge_segments is True           # default on
    e = tmp_path / "e.yaml"
    e.write_text(base + "options: {merge_segments: false}\n")
    assert Config.from_yaml(str(e)).options.merge_segments is False          # opt out


# ---- contraction -------------------------------------------------------------------
def test_contract_chains_merges_same_road_across_osm_id():
    con = _graph()
    GraphSimplifier(con)._contract_chains()
    rows = con.execute("SELECT source, target, osm_id, refs, length_m, ST_NPoints(geometry) "
                       "FROM simplified_edges_forward WHERE source = 1").fetchall()
    assert len(rows) == 1
    s, t, osm, refs, length, npts = rows[0]
    assert (s, t) == (1, 3)                         # outer endpoints of the chain
    assert osm == 100                               # first (source-end) member's osm_id is the representative
    assert refs == [1, 2, 3]                        # stitched node order, shared node not duplicated
    assert npts == 3                                # geometry stitched through the middle node
    assert length == pytest.approx(333585, rel=0.01)  # member lengths summed
    assert con.execute("SELECT count(*) FROM simplified_edges_forward").fetchone()[0] == 5  # 6 -> 5


def test_contract_chains_respects_attribute_and_loop_boundaries():
    con = _graph()
    GraphSimplifier(con)._contract_chains()
    # maxspeed mismatch -> both segments survive
    assert con.execute("SELECT count(*) FROM simplified_edges_forward "
                       "WHERE osm_id IN (300, 301)").fetchone()[0] == 2
    # virtual loop midpoint -> both halves survive
    assert con.execute("SELECT count(*) FROM simplified_edges_forward "
                       "WHERE osm_id = 400").fetchone()[0] == 2


# ---- orientation: source/target match the road's geometry --------------------------
def test_contract_chains_oneway_keeps_legal_direction():
    """Regression: a one-way chain must merge in its LEGAL travel direction (52 -> 50), not be
    flipped to the lower node id (50 -> 52) the way the old `start < cur` orientation did. A
    one-way road has no reverse edge, so a reversed forward edge would be undrivable/wrong-way."""
    con = _oneway_graph()
    GraphSimplifier(con)._contract_chains()
    rows = con.execute("SELECT source, target, osm_id, refs FROM simplified_edges_forward").fetchall()
    assert len(rows) == 1
    s, t, osm, refs = rows[0]
    assert (s, t) == (52, 50)                       # legal one-way direction, NOT (50, 52)
    assert refs == [52, 51, 50]                     # stitched in legal order
    assert osm == 500                               # first (source-end) member's osm_id


def test_merged_edge_geometry_matches_source_and_target():
    """source/target must be consistent with the road's geometry: every merged edge's geometry
    starts on its source node and ends on its target node. Checked for both a two-way and a
    one-way chain (the one-way case also exercises the legal-direction orientation)."""
    for con in (_graph(), _oneway_graph()):
        GraphSimplifier(con)._contract_chains()
        mismatched = con.execute("""
            SELECT e.source, e.target
            FROM simplified_edges_forward e
            JOIN raw.nodes s ON s.osm_id = e.source
            JOIN raw.nodes t ON t.osm_id = e.target
            WHERE ABS(ST_X(ST_StartPoint(e.geometry)) - s.lon) > 1e-9
               OR ABS(ST_Y(ST_StartPoint(e.geometry)) - s.lat) > 1e-9
               OR ABS(ST_X(ST_EndPoint(e.geometry))   - t.lon) > 1e-9
               OR ABS(ST_Y(ST_EndPoint(e.geometry))   - t.lat) > 1e-9
        """).fetchall()
        assert mismatched == []


# ---- matching table ----------------------------------------------------------------
def test_contract_chains_edge_id_map():
    con = _graph()
    GraphSimplifier(con)._contract_chains()
    n, n_new, n_old = con.execute(
        "SELECT count(*), count(DISTINCT new_edge_id), count(DISTINCT old_edge_id) "
        "FROM edge_id_map").fetchone()
    assert (n, n_new, n_old) == (4, 2, 4)           # 2 fwd + 2 rev members -> 1 fwd + 1 rev merged edge
    assert {r[0] for r in con.execute("SELECT DISTINCT seq FROM edge_id_map").fetchall()} == {1, 2}
    # new ids are the same stable hash _rekey_edges will assign to the merged edge
    # (osm_id 100 = the first/source-end member, the representative)
    fwd_new = con.execute("SELECT (hash(100::BIGINT, 1::BIGINT, 3::BIGINT, FALSE) >> 1)::BIGINT").fetchone()[0]
    rev_new = con.execute("SELECT (hash(100::BIGINT, 3::BIGINT, 1::BIGINT, TRUE) >> 1)::BIGINT").fetchone()[0]
    assert {r[0] for r in con.execute("SELECT DISTINCT new_edge_id FROM edge_id_map").fetchall()} \
        == {fwd_new, rev_new}
    # the first forward segment maps from its own merge-off id at seq 1
    old_seg1 = con.execute("SELECT (hash(100::BIGINT, 1::BIGINT, 2::BIGINT, FALSE) >> 1)::BIGINT").fetchone()[0]
    assert con.execute("SELECT count(*) FROM edge_id_map "
                       "WHERE old_edge_id = ? AND seq = 1 AND NOT is_reverse",
                       [old_seg1]).fetchone()[0] == 1


def test_contract_chains_no_candidates_writes_empty_map():
    """No same-road chain -> _contract_chains still creates an (empty) edge_id_map."""
    con = _graph()
    con.execute("DELETE FROM simplified_edges_forward WHERE osm_id IN (100, 200)")  # drop the mergeable chain
    GraphSimplifier(con)._contract_chains()
    assert con.execute("SELECT count(*) FROM edge_id_map").fetchone()[0] == 0
