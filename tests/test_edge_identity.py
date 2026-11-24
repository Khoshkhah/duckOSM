"""
Identity invariants for osm_id / edge_id / edge_ref, and the geometry-preserving splits
(self-loop, same-direction parallel, antiparallel) that make them unique.

Each test writes OSM ways, runs the Rust build (tests/osm_build.py) and checks the tables ``driving.edges`` and
``driving.nodes`` (virtual nodes have ``node_id < 0``), so segmentation, the splits, reverse twins and the re-key all run
for real. See docs/design/split_same_direction_parallels.md. Run: pytest tests/ -q
"""
import math

import duckdb
import pytest
from osm_build import build

ID_HASH = "(hash(osm_id, source, target) >> 1)::BIGINT"


def _build(tmp_path, ways, nodes, mode="driving", extra=None):
    """The Rust build of synthetic ways; a connection to the db.

    ways:  [(osm_id, highway, oneway, [refs...])]
    nodes: {node_id: (lon, lat)}
    """
    n = len(list(tmp_path.glob("e*.duckdb")))
    osm_ways = {i: (refs, {"highway": hw, **({"oneway": "yes"} if oneway else {})}) for i, hw, oneway, refs in ways}
    db = build(tmp_path, nodes, osm_ways, modes=(mode,), name=f"e{n}")
    con = duckdb.connect(str(db), read_only=True)
    con.execute("LOAD spatial")
    return con


def _m(a, b):
    """The distance in metres between two (lon, lat) points (haversine)."""
    (x1, y1), (x2, y2) = ((math.radians(v) for v in p) for p in (a, b))
    h = math.sin((y2 - y1) / 2) ** 2 + math.cos(y1) * math.cos(y2) * math.sin((x2 - x1) / 2) ** 2
    return 2 * 6371008.8 * math.asin(math.sqrt(h))


def _assert_identity_invariants(con):
    """The invariants every build must satisfy, whatever the input topology."""
    one = lambda q: con.execute(q).fetchone()[0]
    n = one("SELECT count(*) FROM driving.edges")
    assert n == one("SELECT count(*) FROM (SELECT DISTINCT osm_id, source, target FROM driving.edges)")
    assert n == one("SELECT count(DISTINCT edge_id) FROM driving.edges")
    assert n == one("SELECT count(DISTINCT edge_ref) FROM driving.edges")
    # edge_id: the direction-free content hash, always positive
    assert one(f"SELECT count(*) FROM driving.edges WHERE edge_id <> {ID_HASH}") == 0
    assert one("SELECT count(*) FROM driving.edges WHERE edge_id <= 0") == 0
    # every edge geometry is valid — a split cut landing on a vertex leaves a zero-length lead
    # segment, which made a subsequent ST_LineSubstring emit a `-nan -nan` vertex (invalid geometry).
    assert one("SELECT count(*) FROM driving.edges WHERE NOT ST_IsValid(geometry) "
               "OR ST_AsText(geometry) LIKE '%nan%'") == 0, "invalid / NaN edge geometry"
    # length_m is the length of the edge's own geometry (the splits once gave both halves of a bent
    # edge the same length_m / 2: up to 23 % off); sphere vs ellipsoid explains < 0.5 %
    assert one("""SELECT count(*) FROM driving.edges WHERE abs(length_m / NULLIF(
        ST_Length_Spheroid(ST_FlipCoordinates(geometry)), 0) - 1) > 0.01""") == 0, "length_m off its geometry"
    # edge_ref parses back: osm_id prefix, f/r suffix matches is_reverse
    assert one("""SELECT count(*) FROM driving.edges WHERE
        split_part(edge_ref, '#', 1) <> osm_id::VARCHAR
        OR right(edge_ref, 1) <> (CASE WHEN is_reverse THEN 'r' ELSE 'f' END)
        OR NOT regexp_matches(edge_ref, '^-?\\d+#\\d+[fr]$')""") == 0
    # a reverse edge shares its seq with the forward twin (swapped endpoints)
    assert one("""SELECT count(*) FROM driving.edges r WHERE r.is_reverse AND NOT EXISTS (
        SELECT 1 FROM driving.edges f WHERE NOT f.is_reverse
          AND f.osm_id = r.osm_id AND f.source = r.target AND f.target = r.source
          AND regexp_replace(f.edge_ref, '[fr]$', '') =
              regexp_replace(r.edge_ref, '[fr]$', ''))""") == 0
    # nodes: unique ids, and every edge endpoint present
    assert one("SELECT count(*) FROM driving.nodes") == one("SELECT count(DISTINCT node_id) FROM driving.nodes")
    assert one("""SELECT count(*) FROM (
        SELECT source AS nid FROM driving.edges UNION SELECT target FROM driving.edges) u
        WHERE nid NOT IN (SELECT node_id FROM driving.nodes)""") == 0


def _assert_no_ref_lost(con, ways, nodes):
    """Every node of every input way still appears in some forward edge's refs — i.e. no arc's
    geometry was deleted (the topological form of length conservation) — and the forward edges of each
    way are as long as the way (the length of the way is conserved, 1 %)."""
    for osm_id, _, _, refs in ways:
        kept = {n for (rs,) in con.execute("SELECT refs FROM driving.edges WHERE osm_id = ? AND NOT is_reverse", [osm_id]).fetchall() for n in rs}
        assert set(refs) <= kept, (osm_id, set(refs) - kept)
        total = sum(_m(nodes[a], nodes[b]) for a, b in zip(refs, refs[1:]))
        got = con.execute("SELECT sum(length_m) FROM driving.edges WHERE osm_id = ? AND NOT is_reverse", [osm_id]).fetchone()[0]
        assert got == pytest.approx(total, rel=0.01), (osm_id, got, total)


# A figure-8: one way visiting junction A(=1) and B(=2) twice, giving three arcs —
# ① A->B (mid length), ② B->A (shortest), ③ A->B (longest). ① and ③ are a same-direction
# parallel pair; each is antiparallel to ②. Modelled on Burnaby way 586121493.
O = (7.0, 43.0)   # the node coordinates are small offsets from here
_at = lambda d: {k: (O[0] + v[0], O[1] + v[1]) for k, v in d.items()}
FIG8_NODES = _at({1: (0.0, 0.0), 2: (0.001, 0.0),
                  11: (0.0003, 0.0002), 12: (0.0007, 0.0002),          # arc ① interior
                  21: (0.0005, -0.0001),                               # arc ② interior
                  31: (0.0002, 0.0006), 32: (0.0005, 0.0008), 33: (0.0008, 0.0006)})  # arc ③
FIG8_REFS = [1, 11, 12, 2, 21, 1, 31, 32, 33, 2]


def test_figure8_two_way_splits_not_deletes(tmp_path):
    ways = [(8, "service", False, FIG8_REFS)]
    con = _build(tmp_path, ways, FIG8_NODES)
    _assert_identity_invariants(con)
    _assert_no_ref_lost(con, ways, FIG8_NODES)
    # ③ split by the parallel step, ① by the antiparallel step, ② whole:
    # 5 forward edges + 5 reverse twins, joined by exactly 2 DISTINCT virtual nodes.
    assert con.execute("SELECT count(*) FROM driving.edges WHERE NOT is_reverse").fetchone()[0] == 5
    assert con.execute("SELECT count(*) FROM driving.edges").fetchone()[0] == 10
    assert con.execute("SELECT count(DISTINCT node_id) FROM driving.nodes WHERE node_id < 0").fetchone()[0] == 2


def test_figure8_one_way_same_direction_split(tmp_path):
    # Same topology, oneway — the two A->B arcs collide as forwards directly (no reverse
    # twins involved), so the parallel split must fire regardless of oneway.
    ways = [(9, "service", True, FIG8_REFS)]
    con = _build(tmp_path, ways, FIG8_NODES)
    _assert_identity_invariants(con)
    _assert_no_ref_lost(con, ways, FIG8_NODES)
    # longest A->B arc split at a virtual node; no reverse edges at all
    assert con.execute("SELECT count(*) FROM driving.edges").fetchone()[0] == 4
    assert con.execute("SELECT count(*) FROM driving.edges WHERE is_reverse").fetchone()[0] == 0
    assert con.execute("SELECT count(DISTINCT node_id) FROM driving.nodes WHERE node_id < 0").fetchone()[0] == 1


def test_classic_lollipop_keeps_loop_geometry(tmp_path):
    # stick 50-51-52 + closed loop 52..52: the self-loop splits at a virtual midpoint, then
    # its two equal halves are an antiparallel pair -> one half splits again. All geometry kept.
    nodes = _at({50: (0.0, 0.0), 51: (0.0003, 0.0), 52: (0.0006, 0.0),
                 61: (0.0008, 0.0002), 62: (0.001, 0.0), 63: (0.0008, -0.0002)})
    ways = [(10, "residential", False, [50, 51, 52, 61, 62, 63, 52])]
    con = _build(tmp_path, ways, nodes)
    _assert_identity_invariants(con)
    _assert_no_ref_lost(con, ways, nodes)
    assert con.execute("SELECT count(*) FROM driving.edges WHERE source = target").fetchone()[0] == 0


def test_lollipop_midpoint_cut_on_vertex_no_nan_geometry(tmp_path):
    # Real failing case — Nacka OSM way 324885236 ("Trossvägen"): a lollipop whose self-loop split
    # cuts exactly on an existing vertex, leaving a zero-length lead segment; the follow-on
    # antiparallel split's ST_LineSubstring then emitted a `-nan -nan` vertex (invalid geometry).
    # These are the way's real coordinates; the validity invariant must hold (ST_RemoveRepeatedPoints).
    nodes = {1: (18.2594068, 59.2876533), 2: (18.2593757, 59.2878397), 3: (18.2593702, 59.2880503),
             4: (18.259103, 59.2880485), 5: (18.2591085, 59.2878379)}
    ways = [(324885236, "service", False, [1, 2, 3, 4, 5, 2])]
    con = _build(tmp_path, ways, nodes)
    _assert_identity_invariants(con)   # includes the no-NaN / valid-geometry assertion
    _assert_no_ref_lost(con, ways, nodes)


def test_pure_closed_loop_two_way_no_nan_geometry(tmp_path):
    # Real failing case — Vancouver OSM way 1303298781 ("parking_aisle"): a PURE closed loop
    # (first ref == last, no stick — the whole way is a self-loop A-B-C-D-A). It is tagged
    # oneway=yes, but WALKING drops oneway (pedestrians go both ways), so its two self-loop
    # halves become an antiparallel pair and one is split again. That second ST_LineSubstring
    # emitted a `-nan -nan` vertex, which reprojected to (inf, inf) on export. Build it as the
    # walking graph sees it (two-way); the no-NaN / valid-geometry invariant must hold.
    nodes = {1: (-123.1309396, 49.2632661), 2: (-123.1309456, 49.2631048),
             3: (-123.1303185, 49.263094800000005), 4: (-123.1303125, 49.2632561)}
    ways = [(1303298781, "service", False, [1, 2, 3, 4, 1])]
    con = _build(tmp_path, ways, nodes)
    _assert_identity_invariants(con)   # includes the no-NaN / valid-geometry assertion
    _assert_no_ref_lost(con, ways, nodes)
    # the antiparallel re-split of a loop half yields an edge between two virtual nodes (fwd+rev)
    assert con.execute("SELECT count(*) FROM driving.edges WHERE source < 0 AND target < 0").fetchone()[0] == 2


def test_two_loops_same_anchor_get_distinct_virtual_nodes(tmp_path):
    # One way anchoring TWO closed loops at the SAME node: the refs-salted virtual ids must
    # differ (the old -(hash(osm_id, source)) collided here -> duplicate node_id).
    nodes = _at({70: (0.0, 0.0),
                 71: (0.0002, 0.0002), 72: (0.0004, 0.0), 73: (0.0002, -0.0002),
                 81: (-0.0002, 0.0002), 82: (-0.0004, 0.0), 83: (-0.0002, -0.0002)})
    ways = [(11, "service", False, [70, 71, 72, 73, 70, 81, 82, 83, 70])]
    con = _build(tmp_path, ways, nodes)
    _assert_identity_invariants(con)
    _assert_no_ref_lost(con, ways, nodes)
    assert con.execute("SELECT count(*) FROM driving.nodes GROUP BY node_id HAVING count(*) > 1").fetchall() == []


def test_path_connector_synthetic_ids(tmp_path):
    # A dangling footway end 6-7 m from a road node: the connector edge must carry
    # osm_id = -min(way ids), a positive hash edge_id, and a well-formed paired edge_ref.
    # Way ids 700 (road) and 800 (footway): the connector is -700. Walking mode, where the connector is built.
    nodes = _at({71: (0.0, 0.0), 72: (0.0002, 0.0),                  # road
                 81: (0.0004, 0.0003), 82: (0.00024, 0.00004)})      # footway, 82 dangles near 72
    con = _build(tmp_path, [(700, "residential", False, [71, 72]), (800, "footway", False, [81, 82])], nodes, mode="walking")
    conn = con.execute("""SELECT edge_id, edge_ref, source, target, osm_id, is_reverse
                          FROM walking.edges WHERE osm_id < 0 ORDER BY is_reverse""").fetchall()
    assert len(conn) == 2                                   # one connector, both directions
    assert {c[4] for c in conn} == {-700}                   # -min(800, 700)
    assert {c[1] for c in conn} == {"-700#1f", "-700#1r"}
    assert all(c[0] > 0 for c in conn)                      # edge_id positive despite osm_id < 0
    assert con.execute(f"SELECT count(*) FROM walking.edges WHERE edge_id <> {ID_HASH}").fetchone()[0] == 0   # the standard hash, all edges
    assert con.execute("SELECT count(*) FROM (SELECT edge_ref FROM walking.edges GROUP BY edge_ref HAVING count(*) > 1)").fetchone()[0] == 0
