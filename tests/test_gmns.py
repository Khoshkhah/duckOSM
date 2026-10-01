"""Tests for duckosm.gmns.to_gmns — extract a built network to a standalone GMNS DuckDB.

Builds a tiny source db (with raw OSM tags, a signal, lane tags, parking) on disk, runs the
extractor, and checks the GMNS tables: link_id=edge_id, referential integrity, per-lane detail from
OSM tags, movement turn types with the immediate-reversal U-turn dropped (kept at a dead end),
signals, and curb.
"""
import duckdb
import pytest

pytest.importorskip("pandas")
from duckosm.gmns import to_gmns, to_meso, to_micro  # noqa: E402

A, B, AR = 6141068311830699705, 3843102655846694531, 1234567890123456789


def _source(path):
    """A 3-node network: A(1->2) two-lane primary with turn/bike lanes, B(2->3) turning north, and
    AR(2->1) the reverse of A. Signal at node 2; parking on way 100."""
    con = duckdb.connect(str(path))
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA raw; CREATE SCHEMA driving")
    con.execute("CREATE TABLE raw.nodes(osm_id BIGINT, lat DOUBLE, lon DOUBLE, tags MAP(VARCHAR,VARCHAR))")
    con.execute("INSERT INTO raw.nodes VALUES (2, 59.32, 18.07, MAP{'highway':'traffic_signals'})")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR,VARCHAR), refs BIGINT[])")
    con.execute("INSERT INTO raw.ways VALUES "
                "(100, MAP{'turn:lanes':'through|right','bicycle:lanes':'no|designated',"
                "'parking:right':'lane'}, [1,2]), (101, MAP{}, [2,3])")
    p = lambda w: f"ST_GeomFromText('{w}')"
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY)")
    con.execute(f"INSERT INTO driving.nodes VALUES (1,{p('POINT(18.06 59.32)')}),"
                f"(2,{p('POINT(18.07 59.32)')}),(3,{p('POINT(18.07 59.31)')})")  # 3 is south of 2
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, osm_id BIGINT, "
                "highway VARCHAR, name VARCHAR, lanes INTEGER, is_reverse BOOLEAN, length_m FLOAT, "
                "maxspeed_kmh FLOAT, geometry GEOMETRY)")
    con.execute(f"""INSERT INTO driving.edges VALUES
        ({A},1,2,100,'primary','Main',2,false,80,50,{p('LINESTRING(18.06 59.32,18.07 59.32)')}),
        ({B},2,3,101,'residential',NULL,1,false,110,30,{p('LINESTRING(18.07 59.32,18.07 59.31)')}),
        ({AR},2,1,100,'primary','Main',2,true,80,50,{p('LINESTRING(18.07 59.32,18.06 59.32)')})""")
    con.execute("CREATE TABLE driving.edge_graph(from_edge BIGINT, to_edge BIGINT, via_edge BIGINT, cost DOUBLE)")
    con.execute(f"INSERT INTO driving.edge_graph VALUES ({A},{B},{B},1.0),({A},{AR},{AR},1.0)")  # A->AR = U-turn
    con.close()


def _gmns(tmp_path, **kw):
    src = tmp_path / "src.duckdb"
    _source(src)
    out = tmp_path / "out_gmns.duckdb"
    to_gmns(str(src), str(out), **kw)
    con = duckdb.connect(str(out))
    con.execute("LOAD spatial;")
    return con


def test_all_tables_and_link_id_is_edge_id(tmp_path):
    con = _gmns(tmp_path)
    tables = {r[0] for r in con.execute(
        "SELECT table_name FROM duckdb_tables() WHERE schema_name='gmns_driving'").fetchall()}
    assert {"config", "node", "link", "geometry", "lane", "movement", "use_definition",
            "use_group", "signal_controller", "curb_seg"} <= tables
    ids = {r[0] for r in con.execute("SELECT link_id FROM gmns_driving.link").fetchall()}
    assert ids == {A, B, AR}                                     # link_id == edge_id, exact


def test_referential_integrity(tmp_path):
    con = _gmns(tmp_path)
    q = lambda s: con.execute(s).fetchone()[0]
    assert q("SELECT count(*) FROM gmns_driving.link WHERE from_node_id NOT IN (SELECT node_id FROM gmns_driving.node)") == 0
    assert q("SELECT count(*) FROM gmns_driving.lane WHERE link_id NOT IN (SELECT link_id FROM gmns_driving.link)") == 0
    assert q("SELECT count(*) FROM gmns_driving.movement WHERE ib_link_id NOT IN (SELECT link_id FROM gmns_driving.link)") == 0


def test_lane_detail_from_osm_tags(tmp_path):
    con = _gmns(tmp_path)
    lanes = con.execute(
        f"SELECT lane_num, turn, allowed_uses, geom IS NOT NULL FROM gmns_driving.lane "
        f"WHERE link_id={A} ORDER BY lane_num").fetchall()
    assert len(lanes) == 2                                       # two-lane link
    assert lanes[0][1] == "through" and lanes[1][1] == "right"   # turn:lanes = through|right
    assert lanes[1][2] == "bike"                                 # bicycle:lanes = no|designated -> lane 2 bike
    assert all(l[3] for l in lanes)                              # offset geometry present


def test_movement_drops_immediate_uturn_and_types_turns(tmp_path):
    """A->AR is the immediate reversal: dropped when A has another way on (B) and AR another way in
    (C, a road from node 3 into node 2). A->B (turning north) is kept and typed."""
    src = tmp_path / "src.duckdb"
    _source(src)
    c = duckdb.connect(str(src))
    c.execute("LOAD spatial; INSERT INTO driving.edges VALUES (77,3,2,102,'residential',NULL,1,false,110,30,"
              "ST_GeomFromText('LINESTRING(18.07 59.31,18.07 59.32)'))")
    c.execute(f"INSERT INTO driving.edge_graph VALUES (77,{AR},{AR},1.0)")
    c.close()
    out = tmp_path / "out_gmns.duckdb"
    to_gmns(str(src), str(out))
    con = duckdb.connect(str(out))
    assert con.execute(
        f"SELECT count(*) FROM gmns_driving.movement WHERE ib_link_id={A} AND ob_link_id={AR}").fetchone()[0] == 0
    row = con.execute(
        f"SELECT type, ctrl_type FROM gmns_driving.movement WHERE ib_link_id={A} AND ob_link_id={B}").fetchone()
    assert row is not None and row[0] in ("left", "right")       # a real turn at the junction
    assert row[1] == "signal"                                    # node 2 is signalized


def test_movement_keeps_the_uturn_that_is_the_only_way_in(tmp_path):
    """Node 2: A arrives, AR and B leave, nothing else arrives. A can go on (to B), but the U-turn is
    the only way into AR, so it's kept; without it AR's lanes can't be reached (Monaco, 10 links)."""
    row = _gmns(tmp_path).execute(
        f"SELECT type FROM gmns_driving.movement WHERE ib_link_id={A} AND ob_link_id={AR}").fetchone()
    assert row == ("uturn",)


def test_movement_keeps_the_uturn_at_a_dead_end(tmp_path):
    """Where turning round is the only way on (A has no other turn), the reversal is kept as a
    'uturn' movement; lane-level routing is stuck at a dead end without it."""
    src = tmp_path / "src.duckdb"
    _source(src)
    c = duckdb.connect(str(src))
    c.execute(f"DELETE FROM driving.edge_graph WHERE from_edge = {A} AND to_edge = {B}")
    c.close()
    out = tmp_path / "out_gmns.duckdb"
    to_gmns(str(src), str(out))
    row = duckdb.connect(str(out)).execute(
        f"SELECT type FROM gmns_driving.movement WHERE ib_link_id={A} AND ob_link_id={AR}").fetchone()
    assert row == ("uturn",)


def test_signal_and_curb(tmp_path):
    con = _gmns(tmp_path)
    assert con.execute("SELECT ctrl_type FROM gmns_driving.node WHERE node_id=2").fetchone()[0] == "signal"
    assert con.execute("SELECT count(*) FROM gmns_driving.signal_controller").fetchone()[0] == 1
    assert con.execute("SELECT count(*) FROM gmns_driving.curb_seg WHERE regulation='lane'").fetchone()[0] >= 1


def test_meso_network(tmp_path):
    from duckosm.gmns import to_meso
    src = tmp_path / "src.duckdb"
    _source(src)
    out = tmp_path / "out_gmns.duckdb"
    to_gmns(str(src), str(out))
    to_meso(str(out), modes=["driving"])

    con = duckdb.connect(str(out))
    con.execute("LOAD spatial;")
    q = lambda s: con.execute(s).fetchone()[0]
    assert q("SELECT count(*) FROM meso_driving.meso_node") == 3 * 2          # 2 meso nodes per macro link
    assert q("SELECT count(*) FROM meso_driving.meso_link WHERE meso_type='normal'") == 3
    # one connector per legal movement (the A->AR U-turn was already dropped upstream)
    assert (q("SELECT count(*) FROM meso_driving.meso_link WHERE meso_type='movement'")
            == q("SELECT count(*) FROM gmns_driving.movement"))
    # referential integrity: every endpoint resolves to a meso node
    assert q("SELECT count(*) FROM meso_driving.meso_link l "
             "WHERE from_node_id NOT IN (SELECT node_id FROM meso_driving.meso_node)") == 0
    # reversible ids: 'M'||macro_link_id == the section link_id
    assert q("SELECT count(*) FROM meso_driving.meso_link "
             "WHERE meso_type='normal' AND 'M'||macro_link_id::VARCHAR = link_id") == 3
    # turn:lanes drives the connector's lane range: A->B is a right turn, and A's lane 2 is the 'right' lane
    lanes, s_ib, e_ib = con.execute(
        f"SELECT lanes, start_ib_lane, end_ib_lane FROM meso_driving.meso_link "
        f"WHERE link_id = 'X{A}-{B}'").fetchone()
    assert (lanes, s_ib, e_ib) == (1, 2, 2)


def test_capacity_and_movement_enrichment(tmp_path):
    src = tmp_path / "src.duckdb"
    _source(src)
    out = tmp_path / "out_gmns.duckdb"
    to_gmns(str(src), str(out))
    con = duckdb.connect(str(out))
    con.execute("LOAD spatial;")
    # capacity default by facility_type: A is 'primary' -> 1600 pce/hr/lane
    assert con.execute(f"SELECT capacity FROM gmns_driving.link WHERE link_id={A}").fetchone()[0] == 1600
    # movement A->B: A heads east and turns south (right) -> mvmt_code EBR; the right lane (2) feeds it
    r = con.execute(f"SELECT mvmt_code, start_ib_lane, end_ib_lane, geometry "
                    f"FROM gmns_driving.movement WHERE ib_link_id={A} AND ob_link_id={B}").fetchone()
    assert r[0] == "EBR" and (r[1], r[2]) == (2, 2) and r[3] is not None
    # every movement gets a code + a connector geometry
    n, coded = con.execute("SELECT count(*), count(mvmt_code) FROM gmns_driving.movement").fetchone()
    assert coded == n and n > 0


def test_cycling_meso(tmp_path):
    """Meso builds for cycling too (same code path as driving), not just the default."""
    src = tmp_path / "src.duckdb"
    _two_mode_source(src)
    out = tmp_path / "out_gmns.duckdb"
    to_gmns(str(src), str(out), modes=["driving", "cycling"])
    to_meso(str(out), modes=["driving", "cycling"])
    con = duckdb.connect(str(out))
    for m in ("driving", "cycling"):
        assert con.execute(
            f"SELECT count(*) FROM meso_{m}.meso_link WHERE meso_type='normal'").fetchone()[0] == 2


def test_micro_network(tmp_path):
    src = tmp_path / "src.duckdb"
    _source(src)
    out = tmp_path / "out_gmns.duckdb"
    to_gmns(str(src), str(out))
    to_micro(str(out), modes=["driving"], cell_length_m=7.0)
    con = duckdb.connect(str(out))
    con.execute("LOAD spatial;")
    q = lambda s: con.execute(s).fetchone()[0]
    assert q("SELECT count(*) FROM micro_driving.micro_link WHERE cell_type='normal'") > 0   # cells
    assert q(f"SELECT count(*) FROM micro_driving.micro_link "                                # lane change on A (2 lanes)
             f"WHERE cell_type='lane_change' AND macro_link_id={A}") > 0
    assert q("SELECT count(*) FROM micro_driving.micro_link WHERE cell_type='movement'") > 0  # turn connector
    # referential integrity + reversible cell ids
    assert q("SELECT count(*) FROM micro_driving.micro_link l "
             "WHERE from_node_id NOT IN (SELECT node_id FROM micro_driving.micro_node)") == 0
    assert q("SELECT count(*) FROM micro_driving.micro_link "
             "WHERE cell_type='normal' AND link_id NOT LIKE 'C%'") == 0


def test_two_way_lanes_offset_to_sides(tmp_path):
    """Drive-side offset: the two directions of a road get their lanes on opposite sides (not
    overlapping on the centerline)."""
    src = tmp_path / "src.duckdb"
    _source(src)
    out = tmp_path / "out_gmns.duckdb"
    to_gmns(str(src), str(out))
    con = duckdb.connect(str(out))
    con.execute("LOAD spatial;")
    # A (1->2) and AR (2->1) are the two directions of the same road
    d = con.execute(f"""SELECT ST_Distance(
        (SELECT geom FROM gmns_driving.lane WHERE link_id={A} AND lane_num=1),
        (SELECT geom FROM gmns_driving.lane WHERE link_id={AR} AND lane_num=1)) * 111320""").fetchone()[0]
    assert d > 1.0                                              # separated on the ground (was 0 = overlapping)


def _two_mode_source(path):
    """A source with driving + cycling sharing edges A, B (same edge_ids across modes)."""
    con = duckdb.connect(str(path))
    con.execute("INSTALL spatial; LOAD spatial;")
    p = lambda w: f"ST_GeomFromText('{w}')"
    for m in ("driving", "cycling"):
        con.execute(f"CREATE SCHEMA {m}")
        con.execute(f"CREATE TABLE {m}.nodes(node_id BIGINT, geom GEOMETRY)")
        con.execute(f"INSERT INTO {m}.nodes VALUES (1,{p('POINT(18.06 59.32)')}),"
                    f"(2,{p('POINT(18.07 59.32)')}),(3,{p('POINT(18.07 59.31)')})")
        con.execute(f"CREATE TABLE {m}.edges(edge_id BIGINT, source BIGINT, target BIGINT, osm_id BIGINT, "
                    f"highway VARCHAR, name VARCHAR, lanes INTEGER, is_reverse BOOLEAN, length_m FLOAT, "
                    f"maxspeed_kmh FLOAT, geometry GEOMETRY)")
        con.execute(f"""INSERT INTO {m}.edges VALUES
            ({A},1,2,100,'primary','Main',1,false,80,50,{p('LINESTRING(18.06 59.32,18.07 59.32)')}),
            ({B},2,3,101,'residential',NULL,1,false,110,30,{p('LINESTRING(18.07 59.32,18.07 59.31)')})""")
        con.execute(f"CREATE TABLE {m}.edge_graph(from_edge BIGINT, to_edge BIGINT, via_edge BIGINT, cost DOUBLE)")
        con.execute(f"INSERT INTO {m}.edge_graph VALUES ({A},{B},{B},1.0)")
    con.close()


def test_combined_mode_tagged(tmp_path):
    src = tmp_path / "src.duckdb"
    _two_mode_source(src)
    out = tmp_path / "out_gmns.duckdb"
    to_gmns(str(src), str(out), modes=["driving", "cycling"], combined=True)
    con = duckdb.connect(str(out))
    # A and B are in both modes -> merged to one row each; link_id (= edge_id) unique again
    n, d = con.execute("SELECT count(*), count(DISTINCT link_id) FROM gmns_all.link").fetchone()
    assert n == 2 and d == 2
    # allowed_uses unioned across the modes that contain the edge: auto (driving) + bike (cycling)
    assert con.execute(f"SELECT allowed_uses FROM gmns_all.link WHERE link_id={A}").fetchone()[0] == "auto,bike"
    assert con.execute("SELECT count(*) FROM gmns_all.node").fetchone()[0] == 3   # union of node ids


def test_to_csv_is_spec_clean(tmp_path):
    out_csv = tmp_path / "csv"
    con = _gmns(tmp_path, modes=["driving"], to_csv=str(out_csv))
    con.close()
    import csv
    with open(out_csv / "lane.csv") as f:
        cols = next(csv.reader(f))
    assert "geom" not in cols and "turn" not in cols            # non-spec columns dropped for CSV
    assert cols[:3] == ["lane_id", "link_id", "lane_num"]


def _pair_source(path, gap_m, b_north=True):
    """Two one-way 2-lane edges of one road, mapped as two ways: P (1->2) east, Q (3->4) west,
    ``gap_m`` metres north (or south) of P."""
    con = duckdb.connect(str(path))
    con.execute("INSTALL spatial; LOAD spatial; CREATE SCHEMA raw; CREATE SCHEMA driving")
    con.execute("CREATE TABLE raw.nodes(osm_id BIGINT, lat DOUBLE, lon DOUBLE, tags MAP(VARCHAR,VARCHAR))")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR,VARCHAR), refs BIGINT[])")
    con.execute("INSERT INTO raw.ways VALUES (200, MAP{}, [1,2]), (201, MAP{}, [3,4])")
    y = 59.32 + (1 if b_north else -1) * gap_m / 111320
    p = lambda w: f"ST_GeomFromText('{w}')"
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY)")
    con.execute(f"INSERT INTO driving.nodes VALUES (1,{p('POINT(18.06 59.32)')}),(2,{p('POINT(18.07 59.32)')}),"
                f"(3,{p(f'POINT(18.07 {y})')}),(4,{p(f'POINT(18.06 {y})')})")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, osm_id BIGINT, "
                "highway VARCHAR, name VARCHAR, lanes INTEGER, is_reverse BOOLEAN, oneway BOOLEAN, "
                "length_m FLOAT, maxspeed_kmh FLOAT, geometry GEOMETRY)")
    con.execute(f"""INSERT INTO driving.edges VALUES
        (11,1,2,200,'primary','Road',2,false,true,560,50,{p('LINESTRING(18.06 59.32,18.07 59.32)')}),
        (12,3,4,201,'primary','Road',2,false,true,560,50,{p(f'LINESTRING(18.07 {y},18.06 {y})')})""")
    con.execute("CREATE TABLE driving.edge_graph(from_edge BIGINT, to_edge BIGINT, via_edge BIGINT, cost DOUBLE)")
    con.close()


def _lane_y(tmp_path, gap_m, **kw):
    """Each lane's offset north of P's line, in metres, by lane id."""
    tag = f"{gap_m}_{kw.get('b_north', True)}_{kw.get('drive_side', 'r')}_{kw.get('pair_carriageways', 1)}"
    src, out = tmp_path / f"pair_{tag}.duckdb", tmp_path / f"pair_{tag}_gmns.duckdb"
    _pair_source(src, gap_m, b_north=kw.pop("b_north", True))
    to_gmns(str(src), str(out), **kw)
    con = duckdb.connect(str(out))
    con.execute("LOAD spatial;")
    rows = con.execute("SELECT lane_id, ST_Y(ST_StartPoint(geom)) FROM gmns_driving.lane").fetchall()
    return {k: round((y - 59.32) * 111320, 2) for k, y in rows}


def test_one_way_carriageways_of_one_road_are_placed_as_one_road(tmp_path):
    """docs/design/gmns_paired_carriageways.md: two 2-lane one-way ways 4 m apart (they need 13 m)
    are placed from the line midway between them, 2 m from each: no overlap, lane 1s 3.25 m apart."""
    y = _lane_y(tmp_path, 4.0)
    assert y["11_1"] == pytest.approx(0.375, abs=0.05) and y["11_2"] == pytest.approx(-2.875, abs=0.05)
    assert y["12_1"] == pytest.approx(3.625, abs=0.05) and y["12_2"] == pytest.approx(6.875, abs=0.05)
    assert y["12_1"] - y["11_1"] == pytest.approx(3.25, abs=0.05)


def test_far_apart_or_off_keeps_one_way_lanes_centred(tmp_path):
    assert _lane_y(tmp_path, 20.0)["11_1"] == pytest.approx(1.625, abs=0.05)     # far apart: as before
    assert _lane_y(tmp_path, 4.0, pair_carriageways=False)["11_1"] == pytest.approx(1.625, abs=0.05)


def test_partner_must_be_on_the_inner_side(tmp_path):
    """Right-hand traffic: the opposite direction is on the left; a partner on the right isn't one.
    Left-hand traffic mirrors it."""
    assert _lane_y(tmp_path, 4.0, b_north=False)["11_1"] == pytest.approx(1.625, abs=0.05)
    y = _lane_y(tmp_path, 4.0, b_north=False, drive_side="left")
    assert y["11_1"] == pytest.approx(-0.375, abs=0.05) and y["12_1"] - y["11_1"] == pytest.approx(-3.25, abs=0.05)


def test_turn_lanes_values_feed_every_turn_they_name():
    """docs/design/gmns_lane_movements.md step 1: 'through;slight_right' feeds thru and right (it was
    read as thru only); a slight turn also feeds thru (typed so under 30 degrees)."""
    from duckosm.gmns import _turn_kinds
    assert _turn_kinds("through;slight_right") == {"thru", "right"}
    assert _turn_kinds("through;right") == {"thru", "right"}
    assert _turn_kinds("left") == {"left"} and _turn_kinds("reverse") == {"uturn"}
    assert _turn_kinds("") == {"thru"} and _turn_kinds("none") == {"thru"}


def test_default_lanes_follow_osm2gmns():
    """Steps 2-3, osm2gmns 0.7.6 (autoconintd.py): separate lanes per turn, equal-length ranges
    read in order. Outbound links sorted left to right, 0-based lanes."""
    from duckosm.gmns import _default_lanes
    # 3 lanes, a 4-way junction (left, thru, right; 2 lanes each): left 1, thru 2, right 3
    assert _default_lanes(3, [2, 2, 2]) == [((0, 0), (0, 0)), ((1, 1), (1, 1)), ((2, 2), (1, 1))]
    # 2 lanes, two ways on: the right one gets the right lane, the left one the rest
    assert _default_lanes(2, [2, 2]) == [((0, 0), (0, 0)), ((1, 1), (1, 1))]
    # one way on: as many lanes as both have, from the left, lane k into lane k
    assert _default_lanes(3, [2]) == [((0, 1), (0, 1))]
    # 1 lane: every turn from it; the leftmost link entered at its lane 1, the others at the right
    assert _default_lanes(1, [2, 3, 2]) == [((0, 0), (0, 0)), ((0, 0), (2, 2)), ((0, 0), (1, 1))]
    # 4 lanes, 4 ways on: two middle links share the 2 middle lanes, one each
    assert _default_lanes(4, [1, 1, 1, 1]) == [((0, 0), (0, 0)), ((1, 1), (0, 0)), ((2, 2), (0, 0)), ((3, 3), (0, 0))]


def test_movement_lanes_from_turn_lanes(tmp_path):
    """Way 100 (A) has turn:lanes 'through|right': its right turn into B starts from lane 2 only,
    into B's single lane. Every movement has both ranges, equal length."""
    con = _gmns(tmp_path)
    assert con.execute(f"SELECT start_ib_lane, end_ib_lane, start_ob_lane, end_ob_lane FROM gmns_driving.movement "
                       f"WHERE ib_link_id={A} AND ob_link_id={B}").fetchone() == (2, 2, 1, 1)
    assert con.execute("SELECT count(*) FROM gmns_driving.movement WHERE start_ib_lane IS NULL OR start_ob_lane IS NULL "
                       "OR end_ib_lane - start_ib_lane <> end_ob_lane - start_ob_lane").fetchone()[0] == 0


def test_lane_graph_pairs_movement_lanes_in_order(tmp_path):
    """Step 4: lane routing follows the movement's ranges: A's right turn into B (turn:lanes
    'through|right') leaves from lane 2 only, so the lane graph has A_2 -> B_1 and not A_1 -> B_1."""
    from duckosm.lane_routing import build_lane_graph
    _gmns(tmp_path).close()
    out = tmp_path / "out_gmns.duckdb"
    build_lane_graph(str(out))
    con = duckdb.connect(str(out))
    got = con.execute(f"SELECT from_lane, to_lane FROM lane_driving.lane_edges WHERE kind <> 'lane_change' "
                      f"AND from_lane LIKE '{A}_%' AND to_lane LIKE '{B}_%'").fetchall()
    assert got == [(f"{A}_2", f"{B}_1")]
