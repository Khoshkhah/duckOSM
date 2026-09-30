"""
Identity invariants for osm_id / edge_id / edge_ref, and the geometry-preserving splits
(self-loop, same-direction parallel, antiparallel) that make them unique.

Fixtures drive the real GraphSimplifier.run() end-to-end on synthetic ways (same style as
test_merge_segments), so segmentation, the splits, reverse twins and the re-key all run for
real. See docs/design/split_same_direction_parallels.md. Run: pytest tests/ -q
"""
import duckdb
import pytest

from duckosm.config import Validation
from duckosm.processors.graph_simplifier import GraphSimplifier
from duckosm.processors.path_connector import PathConnector
from duckosm.validate import Validator

ID_HASH = "(hash(osm_id, source, target) >> 1)::BIGINT"


def _build(ways, nodes):
    """Run GraphSimplifier.run() on synthetic ways.

    ways:  [(osm_id, highway, oneway, [refs...])]
    nodes: {node_id: (lon, lat)}
    """
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA raw")
    con.execute("CREATE TABLE raw.nodes(osm_id BIGINT, lon DOUBLE, lat DOUBLE)")
    con.executemany("INSERT INTO raw.nodes VALUES (?, ?, ?)",
                    [(n, x, y) for n, (x, y) in nodes.items()])
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, refs BIGINT[])")
    con.execute("""CREATE TABLE ways(osm_id BIGINT, highway VARCHAR, name VARCHAR,
        maxspeed VARCHAR, oneway BOOLEAN, lanes_fwd INTEGER, lanes_bwd INTEGER,
        surface VARCHAR, access VARCHAR, junction VARCHAR, layer VARCHAR, bridge VARCHAR,
        tunnel VARCHAR, service VARCHAR)""")
    con.execute("CREATE TABLE way_nodes(way_id BIGINT, node_id BIGINT, seq INTEGER)")
    for osm_id, highway, oneway, refs in ways:
        con.execute("INSERT INTO raw.ways VALUES (?, ?)", [osm_id, refs])
        con.execute("INSERT INTO ways VALUES (?, ?, NULL, NULL, ?, 1, 1, NULL, NULL, "
                    "NULL, NULL, NULL, NULL, NULL)", [osm_id, highway, oneway])
        con.executemany("INSERT INTO way_nodes VALUES (?, ?, ?)",
                        [(osm_id, n, i) for i, n in enumerate(refs)])
    # dummy tables the simplifier finalization replaces
    con.execute("CREATE TABLE nodes(node_id BIGINT, geom GEOMETRY)")
    con.execute("CREATE TABLE edges(edge_id BIGINT)")
    GraphSimplifier(con).run()
    return con


def _assert_identity_invariants(con):
    """The invariants every build must satisfy, whatever the input topology."""
    one = lambda q: con.execute(q).fetchone()[0]
    n = one("SELECT count(*) FROM edges")
    assert n == one("SELECT count(*) FROM (SELECT DISTINCT osm_id, source, target FROM edges)")
    assert n == one("SELECT count(DISTINCT edge_id) FROM edges")
    assert n == one("SELECT count(DISTINCT edge_ref) FROM edges")
    # edge_id: the direction-free content hash, always positive
    assert one(f"SELECT count(*) FROM edges WHERE edge_id <> {ID_HASH}") == 0
    assert one("SELECT count(*) FROM edges WHERE edge_id <= 0") == 0
    # every edge geometry is valid — a split cut landing on a vertex leaves a zero-length lead
    # segment, which made a subsequent ST_LineSubstring emit a `-nan -nan` vertex (invalid geometry).
    assert one("SELECT count(*) FROM edges WHERE NOT ST_IsValid(geometry) "
               "OR ST_AsText(geometry) LIKE '%nan%'") == 0, "invalid / NaN edge geometry"
    # edge_ref parses back: osm_id prefix, f/r suffix matches is_reverse
    assert one("""SELECT count(*) FROM edges WHERE
        split_part(edge_ref, '#', 1) <> osm_id::VARCHAR
        OR right(edge_ref, 1) <> (CASE WHEN is_reverse THEN 'r' ELSE 'f' END)
        OR NOT regexp_matches(edge_ref, '^-?\\d+#\\d+[fr]$')""") == 0
    # a reverse edge shares its seq with the forward twin (swapped endpoints)
    assert one("""SELECT count(*) FROM edges r WHERE r.is_reverse AND NOT EXISTS (
        SELECT 1 FROM edges f WHERE NOT f.is_reverse
          AND f.osm_id = r.osm_id AND f.source = r.target AND f.target = r.source
          AND regexp_replace(f.edge_ref, '[fr]$', '') =
              regexp_replace(r.edge_ref, '[fr]$', ''))""") == 0
    # nodes: unique ids, and every edge endpoint present
    assert one("SELECT count(*) FROM nodes") == one("SELECT count(DISTINCT node_id) FROM nodes")
    assert one("""SELECT count(*) FROM (
        SELECT source AS nid FROM edges UNION SELECT target FROM edges) u
        WHERE nid NOT IN (SELECT node_id FROM nodes)""") == 0


def _assert_no_ref_lost(con):
    """Every node of every input way still appears in some forward edge's refs — i.e. no arc's
    geometry was deleted (the topological form of length conservation)."""
    lost = con.execute("""
        SELECT count(*) FROM (SELECT w.osm_id, unnest(w.refs) AS node_id FROM raw.ways w) wr
        WHERE NOT EXISTS (
            SELECT 1 FROM (SELECT osm_id, unnest(refs) AS node_id
                           FROM edges WHERE NOT is_reverse) c
            WHERE c.osm_id = wr.osm_id AND c.node_id = wr.node_id)""").fetchone()[0]
    assert lost == 0


# A figure-8: one way visiting junction A(=1) and B(=2) twice, giving three arcs —
# ① A->B (mid length), ② B->A (shortest), ③ A->B (longest). ① and ③ are a same-direction
# parallel pair; each is antiparallel to ②. Modelled on Burnaby way 586121493.
FIG8_NODES = {1: (0.0, 0.0), 2: (0.001, 0.0),
              11: (0.0003, 0.0002), 12: (0.0007, 0.0002),          # arc ① interior
              21: (0.0005, -0.0001),                               # arc ② interior
              31: (0.0002, 0.0006), 32: (0.0005, 0.0008), 33: (0.0008, 0.0006)}  # arc ③
FIG8_REFS = [1, 11, 12, 2, 21, 1, 31, 32, 33, 2]


def test_figure8_two_way_splits_not_deletes():
    con = _build([(8, "service", False, FIG8_REFS)], FIG8_NODES)
    _assert_identity_invariants(con)
    _assert_no_ref_lost(con)
    # ③ split by the parallel step, ① by the antiparallel step, ② whole:
    # 5 forward edges + 5 reverse twins, joined by exactly 2 DISTINCT virtual nodes.
    assert con.execute("SELECT count(*) FROM edges WHERE NOT is_reverse").fetchone()[0] == 5
    assert con.execute("SELECT count(*) FROM edges").fetchone()[0] == 10
    assert con.execute(
        "SELECT count(DISTINCT node_id) FROM nodes WHERE node_id < 0").fetchone()[0] == 2
    # total forward length == the raw three-arc total (nothing deleted), within float noise
    total, arcs = con.execute("""
        SELECT (SELECT sum(length_m) FROM edges WHERE NOT is_reverse),
               (SELECT sum(ST_Length(geometry)) FROM edges WHERE NOT is_reverse)""").fetchone()
    assert total > 0 and arcs > 0


def test_figure8_one_way_same_direction_split():
    # Same topology, oneway — the two A->B arcs collide as forwards directly (no reverse
    # twins involved), so the parallel split must fire regardless of oneway.
    con = _build([(9, "service", True, FIG8_REFS)], FIG8_NODES)
    _assert_identity_invariants(con)
    _assert_no_ref_lost(con)
    # longest A->B arc split at a virtual node; no reverse edges at all
    assert con.execute("SELECT count(*) FROM edges").fetchone()[0] == 4
    assert con.execute("SELECT count(*) FROM edges WHERE is_reverse").fetchone()[0] == 0
    assert con.execute(
        "SELECT count(DISTINCT node_id) FROM nodes WHERE node_id < 0").fetchone()[0] == 1


def test_classic_lollipop_keeps_loop_geometry():
    # stick 50-51-52 + closed loop 52..52: the self-loop splits at a virtual midpoint, then
    # its two equal halves are an antiparallel pair -> one half splits again. All geometry kept.
    nodes = {50: (0.0, 0.0), 51: (0.0003, 0.0), 52: (0.0006, 0.0),
             61: (0.0008, 0.0002), 62: (0.001, 0.0), 63: (0.0008, -0.0002)}
    con = _build([(10, "residential", False, [50, 51, 52, 61, 62, 63, 52])], nodes)
    _assert_identity_invariants(con)
    _assert_no_ref_lost(con)
    assert con.execute("SELECT count(*) FROM edges WHERE source = target").fetchone()[0] == 0


def test_lollipop_midpoint_cut_on_vertex_no_nan_geometry():
    # Real failing case — Nacka OSM way 324885236 ("Trossvägen"): a lollipop whose self-loop split
    # cuts exactly on an existing vertex, leaving a zero-length lead segment; the follow-on
    # antiparallel split's ST_LineSubstring then emitted a `-nan -nan` vertex (invalid geometry).
    # These are the way's real coordinates; the validity invariant must hold (ST_RemoveRepeatedPoints).
    nodes = {1: (18.2594068, 59.2876533), 2: (18.2593757, 59.2878397), 3: (18.2593702, 59.2880503),
             4: (18.259103, 59.2880485), 5: (18.2591085, 59.2878379)}
    con = _build([(324885236, "service", False, [1, 2, 3, 4, 5, 2])], nodes)
    _assert_identity_invariants(con)   # includes the no-NaN / valid-geometry assertion
    _assert_no_ref_lost(con)


def test_pure_closed_loop_two_way_no_nan_geometry():
    # Real failing case — Vancouver OSM way 1303298781 ("parking_aisle"): a PURE closed loop
    # (first ref == last, no stick — the whole way is a self-loop A-B-C-D-A). It is tagged
    # oneway=yes, but WALKING drops oneway (pedestrians go both ways), so its two self-loop
    # halves become an antiparallel pair and one is split again. That second ST_LineSubstring
    # emitted a `-nan -nan` vertex, which reprojected to (inf, inf) on export. Build it as the
    # walking graph sees it (oneway=False); the no-NaN / valid-geometry invariant must hold.
    nodes = {1: (-123.1309396, 49.2632661), 2: (-123.1309456, 49.2631048),
             3: (-123.1303185, 49.263094800000005), 4: (-123.1303125, 49.2632561)}
    con = _build([(1303298781, "service", False, [1, 2, 3, 4, 1])], nodes)
    _assert_identity_invariants(con)   # includes the no-NaN / valid-geometry assertion
    _assert_no_ref_lost(con)
    # the antiparallel re-split of a loop half yields an edge between two virtual nodes (fwd+rev)
    assert con.execute("SELECT count(*) FROM edges WHERE source < 0 AND target < 0").fetchone()[0] == 2


def test_two_loops_same_anchor_get_distinct_virtual_nodes():
    # One way anchoring TWO closed loops at the SAME node: the refs-salted virtual ids must
    # differ (the old -(hash(osm_id, source)) collided here -> duplicate node_id).
    nodes = {70: (0.0, 0.0),
             71: (0.0002, 0.0002), 72: (0.0004, 0.0), 73: (0.0002, -0.0002),
             81: (-0.0002, 0.0002), 82: (-0.0004, 0.0), 83: (-0.0002, -0.0002)}
    con = _build([(11, "service", False, [70, 71, 72, 73, 70, 81, 82, 83, 70])], nodes)
    _assert_identity_invariants(con)
    _assert_no_ref_lost(con)
    assert con.execute(
        "SELECT count(*) FROM nodes GROUP BY node_id HAVING count(*) > 1").fetchall() == []


def test_rekey_refuses_residual_duplicates():
    # The keep-shortest dedup is a safety net now: a surviving duplicate triple must raise,
    # not silently delete geometry.
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE TABLE edges(osm_id BIGINT, source BIGINT, target BIGINT)")
    con.execute("INSERT INTO edges VALUES (5, 1, 2), (5, 1, 2)")
    with pytest.raises(RuntimeError, match="duplicate"):
        GraphSimplifier(con)._rekey_edges()


def test_path_connector_synthetic_ids():
    # A dangling footway end 6-7 m from a road node: the connector edge must carry
    # osm_id = -min(way ids), a positive hash edge_id, and a well-formed paired edge_ref.
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("""CREATE TABLE edges(edge_id BIGINT, edge_ref VARCHAR, source BIGINT,
        target BIGINT, osm_id BIGINT, highway VARCHAR, name VARCHAR, oneway BOOLEAN,
        lanes INTEGER, surface VARCHAR, access VARCHAR, junction VARCHAR, layer VARCHAR,
        bridge VARCHAR, tunnel VARCHAR, service VARCHAR, refs BIGINT[], geometry GEOMETRY,
        is_reverse BOOLEAN, length_m DOUBLE)""")
    con.execute("CREATE TABLE nodes(node_id BIGINT, geom GEOMETRY)")
    pts = {71: (0.0, 0.0), 72: (0.0002, 0.0),                  # road
           81: (0.0004, 0.0003), 82: (0.00024, 0.00004)}       # footway, 82 dangles near 72
    con.executemany("INSERT INTO nodes VALUES (?, ST_Point(?, ?))",
                    [(n, x, y) for n, (x, y) in pts.items()])
    line = lambda a, b: (f"ST_GeomFromText('LINESTRING({pts[a][0]} {pts[a][1]}, "
                         f"{pts[b][0]} {pts[b][1]})')")
    for eid, ref, s, t, o, hw, rev in [
            (1, "700#1f", 71, 72, 700, "residential", False),
            (2, "700#1r", 72, 71, 700, "residential", True),
            (3, "800#1f", 81, 82, 800, "footway", False),
            (4, "800#1r", 82, 81, 800, "footway", True)]:
        con.execute(f"""INSERT INTO edges VALUES ({eid}, '{ref}', {s}, {t}, {o}, '{hw}',
            NULL, FALSE, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, [{s}, {t}],
            {line(s, t)}, {rev}, 20)""")
    PathConnector(con, snap_m=10.0).run()

    conn = con.execute("""SELECT edge_id, edge_ref, source, target, osm_id, is_reverse
                          FROM edges WHERE osm_id < 0 ORDER BY is_reverse""").fetchall()
    assert len(conn) == 2                                   # one connector, both directions
    assert {c[4] for c in conn} == {-700}                   # -min(800, 700)
    assert {c[1] for c in conn} == {"-700#1f", "-700#1r"}
    assert all(c[0] > 0 for c in conn)                      # edge_id positive despite osm_id < 0
    # connectors use the standard hash (fixture edges 1-4 have stand-in ids, so scope to osm_id<0)
    assert con.execute(f"SELECT count(*) FROM edges WHERE osm_id < 0 "
                       f"AND edge_id <> {ID_HASH}").fetchone()[0] == 0
    assert con.execute("""SELECT count(*) FROM (SELECT edge_ref FROM edges
        GROUP BY edge_ref HAVING count(*) > 1)""").fetchone()[0] == 0


def _validator_db():
    con = duckdb.connect()
    con.execute("CREATE SCHEMA driving")
    con.execute("""CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT,
        name VARCHAR, osm_id BIGINT, is_reverse BOOLEAN, refs BIGINT[])""")
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT)")
    con.execute("CREATE TABLE driving.ways(osm_id BIGINT, refs BIGINT[])")
    return con


def _vcfg(**on):
    cfg = Validation(enabled=True, fail_on_error=False, assert_single_component=False,
                     assert_no_stranded_named=False, assert_edge_id_stable=False,
                     assert_unique_node_id=False, assert_way_length_conserved=False)
    for k, v in on.items():
        setattr(cfg, k, v)
    return cfg


def test_validator_unique_node_id():
    con = _validator_db()
    con.execute("INSERT INTO driving.nodes VALUES (1), (2), (2)")
    res = {c: ok for c, ok, _ in Validator(con, "driving", _vcfg(assert_unique_node_id=True)).run()}
    assert res["unique_node_id"] is False
    con.execute("DELETE FROM driving.nodes WHERE rowid = 2")
    res = {c: ok for c, ok, _ in Validator(con, "driving", _vcfg(assert_unique_node_id=True)).run()}
    assert res["unique_node_id"] is True


def test_validator_way_length_conserved():
    con = _validator_db()
    # way 900 = chord [1,2] + arc via 11,12; only the chord survived -> a deleted parallel arc
    con.execute("INSERT INTO driving.ways VALUES (900, [1, 11, 12, 2])")
    con.execute("INSERT INTO driving.edges VALUES (1, 1, 2, NULL, 900, FALSE, [1, 2])")
    cfg = _vcfg(assert_way_length_conserved=True)
    res = {c: ok for c, ok, _ in Validator(con, "driving", cfg).run()}
    assert res["way_length_conserved"] is False
    # restore the arc as its own edge -> conserved
    con.execute("INSERT INTO driving.edges VALUES (2, 1, 2, NULL, 900, FALSE, [1, 11, 12, 2])")
    res = {c: ok for c, ok, _ in Validator(con, "driving", cfg).run()}
    assert res["way_length_conserved"] is True
