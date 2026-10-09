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
    con.execute("INSERT INTO raw.nodes VALUES (2, 59.32, 18.07, MAP{'highway':'traffic_signals'}), "
                "(4, 59.32, 18.065, MAP{'highway':'crossing'}), (5, 59.30, 18.00, MAP{'highway':'crossing'})")  # 4 lies on A, 5 on no link
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR,VARCHAR), refs BIGINT[])")
    con.execute("INSERT INTO raw.ways VALUES "
                "(100, MAP{'turn:lanes':'through|right','bicycle:lanes':'no|designated',"
                "'parking:right':'lane'}, [1,2]), (101, MAP{'sidewalk':'separate','cycleway':'track'}, [2,3])")
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
    con.execute("CREATE TABLE main.boundary(name VARCHAR, geom GEOMETRY)")
    con.execute(f"INSERT INTO main.boundary VALUES ('Testville', {p('POLYGON((18.0 59.3, 18.1 59.3, 18.1 59.4, 18.0 59.4, 18.0 59.3))')})")
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


def test_every_lane_use_is_defined_and_config_has_the_spec_columns(tmp_path):
    con = _gmns(tmp_path)
    q = lambda s: con.execute(s).fetchall()
    assert not q("SELECT DISTINCT u FROM (SELECT unnest(string_split(allowed_uses, ',')) AS u FROM gmns_driving.lane) "
                 "WHERE u NOT IN (SELECT use FROM gmns_driving.use_definition)")     # the spec: a comma-separated set
    assert {"auto", "bus", "bike"} <= {r[0] for r in q("SELECT use FROM gmns_driving.use_definition")}
    cols = [r[0] for r in q("DESCRIBE gmns_driving.config")]
    assert "currency" in cols and q("SELECT version_number FROM gmns_driving.config") == [(0.97,)]


def test_referential_integrity(tmp_path):
    con = _gmns(tmp_path)
    q = lambda s: con.execute(s).fetchone()[0]
    assert q("SELECT count(*) FROM gmns_driving.link WHERE from_node_id NOT IN (SELECT node_id FROM gmns_driving.node)") == 0
    assert q("SELECT count(*) FROM gmns_driving.lane WHERE link_id NOT IN (SELECT link_id FROM gmns_driving.link)") == 0
    assert q("SELECT count(*) FROM gmns_driving.movement WHERE ib_link_id NOT IN (SELECT link_id FROM gmns_driving.link)") == 0


def test_lane_detail_from_osm_tags(tmp_path):
    con = _gmns(tmp_path)
    lanes = con.execute(
        f"SELECT lane_num, turn, allowed_uses, geom IS NOT NULL, width FROM gmns_driving.lane "
        f"WHERE link_id={A} ORDER BY lane_num").fetchall()
    assert len(lanes) == 2                                       # two-lane link
    assert lanes[0][1] == "through" and lanes[1][1] == "right"   # turn:lanes = through|right
    assert lanes[1][2] == "bike"                                 # bicycle:lanes = no|designated -> lane 2 bike
    assert lanes[1][4] == 1.5                                    # an untagged bike lane's width is stored: one number for every reader
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
    # every movement gets a connector geometry and a code, except a U-turn (the spec's code has no U)
    n, coded, uturns = con.execute("SELECT count(*), count(mvmt_code), count(*) FILTER (type = 'uturn') "
                                   "FROM gmns_driving.movement").fetchone()
    assert n > 0 and coded == n - uturns


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


def _pair_source(path, gap_m, b_north=True, gap_west=None):
    """Two one-way 2-lane edges of one road, mapped as two ways: P (1->2) east, Q (3->4) west,
    ``gap_m`` metres north (or south) of P (``gap_west``: at Q's end, the west one, if the ways converge)."""
    con = duckdb.connect(str(path))
    con.execute("INSTALL spatial; LOAD spatial; CREATE SCHEMA raw; CREATE SCHEMA driving")
    con.execute("CREATE TABLE raw.nodes(osm_id BIGINT, lat DOUBLE, lon DOUBLE, tags MAP(VARCHAR,VARCHAR))")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR,VARCHAR), refs BIGINT[])")
    con.execute("INSERT INTO raw.ways VALUES (200, MAP{}, [1,2]), (201, MAP{}, [3,4])")
    y = 59.32 + (1 if b_north else -1) * gap_m / 111320
    yw = 59.32 + (1 if b_north else -1) * (gap_west if gap_west is not None else gap_m) / 111320
    p = lambda w: f"ST_GeomFromText('{w}')"
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY)")
    con.execute(f"INSERT INTO driving.nodes VALUES (1,{p('POINT(18.06 59.32)')}),(2,{p('POINT(18.07 59.32)')}),"
                f"(3,{p(f'POINT(18.07 {y})')}),(4,{p(f'POINT(18.06 {yw})')})")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, osm_id BIGINT, "
                "highway VARCHAR, name VARCHAR, lanes INTEGER, is_reverse BOOLEAN, oneway BOOLEAN, "
                "length_m FLOAT, maxspeed_kmh FLOAT, geometry GEOMETRY)")
    con.execute(f"""INSERT INTO driving.edges VALUES
        (11,1,2,200,'primary','Road',2,false,true,560,50,{p('LINESTRING(18.06 59.32,18.07 59.32)')}),
        (12,3,4,201,'primary','Road',2,false,true,560,50,{p(f'LINESTRING(18.07 {y},18.06 {yw})')})""")
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


def test_a_gap_that_varies_along_the_road_leaves_no_wedge(tmp_path):
    """Step 2: the two ways converge from 6 m (west) to 2 m (east); the lane 1s of the two directions meet, 3.25 m apart,
    at both ends (with the one median gap they were 3.25 m apart only in the middle)."""
    src, out = tmp_path / "conv.duckdb", tmp_path / "conv_gmns.duckdb"
    _pair_source(src, 2.0, gap_west=6.0)
    to_gmns(str(src), str(out))
    con = duckdb.connect(str(out))
    con.execute("LOAD spatial;")
    y = lambda lane, pt: (con.execute(f"SELECT ST_Y(ST_{pt}Point(geom)) FROM gmns_driving.lane WHERE lane_id = '{lane}'").fetchone()[0] - 59.32) * 111320   # noqa: E731
    assert y("12_1", "End") - y("11_1", "Start") == pytest.approx(3.25, abs=0.15)      # west end (Q ends there)
    assert y("12_1", "Start") - y("11_1", "End") == pytest.approx(3.25, abs=0.15)      # east end


def test_far_apart_or_off_keeps_one_way_lanes_centred(tmp_path):
    assert _lane_y(tmp_path, 20.0)["11_1"] == pytest.approx(1.625, abs=0.05)     # far apart: as before
    assert _lane_y(tmp_path, 4.0, pair_carriageways=False)["11_1"] == pytest.approx(1.625, abs=0.05)


def test_partner_must_be_on_the_inner_side(tmp_path):
    """Right-hand traffic: the opposite direction is on the left; a partner on the right isn't one.
    Left-hand traffic mirrors it."""
    assert _lane_y(tmp_path, 4.0, b_north=False)["11_1"] == pytest.approx(1.625, abs=0.05)
    # Left-hand traffic mirrors it: the lanes lie left of the line between the two ways, and lane 1 is the
    # LEFTMOST lane, so the farthest from that line (tests/test_gmns_values.py): the inner lanes, nearest the
    # line, are the 2nd lanes, 3.25 m apart
    y = _lane_y(tmp_path, 4.0, b_north=False, drive_side="left")
    assert y["11_2"] == pytest.approx(-0.375, abs=0.05) and y["11_1"] == pytest.approx(2.875, abs=0.05)
    assert y["12_2"] == pytest.approx(-3.625, abs=0.05) and y["12_1"] == pytest.approx(-6.875, abs=0.05)
    assert y["11_2"] - y["12_2"] == pytest.approx(3.25, abs=0.05)


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


def test_bus_lane_from_bus_lanes(tmp_path):
    """Step 5: bus:lanes=designated makes a bus lane, as psv:lanes does (Monaco, Boulevard Princesse
    Charlotte: '||designated' with access:lanes '||no')."""
    src = tmp_path / "src.duckdb"
    _source(src)
    c = duckdb.connect(str(src))
    c.execute("UPDATE raw.ways SET tags = MAP{'bus:lanes':'|designated','access:lanes':'|no'} WHERE osm_id = 100")
    c.close()
    out = tmp_path / "out_gmns.duckdb"
    to_gmns(str(src), str(out))
    uses = dict(duckdb.connect(str(out)).execute(
        f"SELECT lane_num, allowed_uses FROM gmns_driving.lane WHERE link_id = {A}").fetchall())
    assert uses == {1: "auto", 2: "bus"}


def test_empty_turn_lane_goes_straight_on(tmp_path):
    """A lane left empty in turn:lanes ('through|') has no arrow, so straight on: the thru movement
    starts from both lanes (Monaco, Boulevard Princesse Charlotte: its bus lane had no turn)."""
    src = tmp_path / "src.duckdb"
    _source(src)
    c = duckdb.connect(str(src))
    c.execute("LOAD spatial; UPDATE raw.ways SET tags = MAP{'turn:lanes':'through|'} WHERE osm_id = 100")
    c.execute("INSERT INTO driving.nodes VALUES (4, ST_GeomFromText('POINT(18.08 59.32)'))")
    c.execute("INSERT INTO driving.edges VALUES (88,2,4,103,'primary','Main',2,false,80,50,"
              "ST_GeomFromText('LINESTRING(18.07 59.32,18.08 59.32)'))")
    c.execute(f"INSERT INTO driving.edge_graph VALUES ({A},88,88,1.0)")
    c.close()
    out = tmp_path / "out_gmns.duckdb"
    to_gmns(str(src), str(out))
    assert duckdb.connect(str(out)).execute(
        f"SELECT start_ib_lane, end_ib_lane FROM gmns_driving.movement WHERE ib_link_id={A} AND ob_link_id=88"
    ).fetchone() == (1, 2)


def test_placement_says_where_the_line_lies(tmp_path):
    """Step 6: placement=right_of:2 on a 2-lane one-way way: the line is the right edge of lane 2, so
    lane 1 sits 4.875 m and lane 2 1.625 m left of it (north, for an eastbound way), not centred."""
    src, out = tmp_path / "pl.duckdb", tmp_path / "pl_gmns.duckdb"
    _pair_source(src, 40.0)                                   # Q far away: no pairing
    c = duckdb.connect(str(src))
    c.execute("UPDATE raw.ways SET tags = MAP{'placement':'right_of:2'} WHERE osm_id = 200")
    c.close()
    to_gmns(str(src), str(out))
    con = duckdb.connect(str(out))
    con.execute("LOAD spatial;")
    y = dict(con.execute("SELECT lane_id, round((ST_Y(ST_StartPoint(geom)) - 59.32) * 111320, 2) "
                         "FROM gmns_driving.lane WHERE link_id = 11").fetchall())
    assert y["11_1"] == pytest.approx(4.875, abs=0.05) and y["11_2"] == pytest.approx(1.625, abs=0.05)


def test_placement_values():
    from duckosm.gmns import _placement
    w = [3.25, 3.25, 3.0]
    assert _placement("left_of:1", w) == 0 and _placement("middle_of:2", w) == 4.875
    assert _placement("right_of:3", w) == 9.5
    assert _placement("transition", w) is None and _placement("left_of:4", w) is None and _placement(None, w) is None


def _source_with(path, tags, edges, graph):
    """_source plus extra edges (id, source, target, osm_id, lanes, is_reverse, wkt) and edge_graph
    rows, way 100 (A) tagged ``tags``; extra nodes 4/5 east and south-east of node 2."""
    _source(path)
    c = duckdb.connect(str(path))
    c.execute("LOAD spatial")
    c.execute(f"UPDATE raw.ways SET tags = MAP{{{', '.join(f'{k!r}:{v!r}' for k, v in tags.items())}}} WHERE osm_id = 100")
    c.execute("INSERT INTO driving.nodes VALUES (4, ST_GeomFromText('POINT(18.08 59.32)')), "
              "(5, ST_GeomFromText('POINT(18.08 59.3185)'))")
    for e, a, b, osm, n, rev, wkt in edges:
        c.execute(f"INSERT INTO driving.edges VALUES ({e},{a},{b},{osm},'primary','Main',{n},{rev},80,50,"
                  f"ST_GeomFromText('{wkt}'))")
    for a, b in graph:
        c.execute(f"INSERT INTO driving.edge_graph VALUES ({a},{b},{b},1.0)")
    c.close()


def _ranges(tmp_path, ib, ob):
    out = tmp_path / "out_gmns.duckdb"
    to_gmns(str(tmp_path / "src.duckdb"), str(out))
    return duckdb.connect(str(out)).execute(
        f"SELECT start_ib_lane, end_ib_lane, start_ob_lane, end_ob_lane FROM gmns_driving.movement "
        f"WHERE ib_link_id={ib} AND ob_link_id={ob}").fetchone()


def test_turn_lanes_apply_where_the_way_ends(tmp_path):
    """Way 100 'through|right' continues past node 2 as edge 88 (the same way): the arrows apply only
    at the way's end, so at node 2 both lanes go on, lane by lane, into 88 (lane 1 had been sent
    into lane 2 and the right lane had no way on: Monaco, Boulevard Charles III)."""
    _source_with(tmp_path / "src.duckdb", {"turn:lanes": "through|right"},
                 [(88, 2, 4, 100, 2, "false", "LINESTRING(18.07 59.32,18.08 59.32)")], [(A, 88)])
    assert _ranges(tmp_path, A, 88) == (1, 2, 1, 2)


def test_arrows_match_exits_by_their_place(tmp_path):
    """At the way's end, a slight right fork (16 degrees, typed thru by its angle) takes the 'right'
    lane and the straight exit the 'through' lane."""
    _source_with(tmp_path / "src.duckdb", {"turn:lanes": "through|right"},
                 [(88, 2, 4, 103, 2, "false", "LINESTRING(18.07 59.32,18.08 59.32)"),
                  (89, 2, 5, 104, 2, "false", "LINESTRING(18.07 59.32,18.08 59.3185)")], [(A, 88), (A, 89)])
    assert _ranges(tmp_path, A, 88)[:2] == (1, 1)                 # straight on from the 'through' lane
    assert _ranges(tmp_path, A, 89)[:2] == (2, 2)                 # the fork from the 'right' lane


def test_slight_fork_is_typed_thru(tmp_path):
    """Guard for the test above: the fork really is typed thru by its angle."""
    _source_with(tmp_path / "src.duckdb", {"turn:lanes": "through|right"},
                 [(88, 2, 4, 103, 2, "false", "LINESTRING(18.07 59.32,18.08 59.32)"),
                  (89, 2, 5, 104, 2, "false", "LINESTRING(18.07 59.32,18.08 59.3185)")], [(A, 88), (A, 89)])
    out = tmp_path / "out_gmns.duckdb"
    to_gmns(str(tmp_path / "src.duckdb"), str(out))
    assert duckdb.connect(str(out)).execute(
        f"SELECT type FROM gmns_driving.movement WHERE ib_link_id={A} AND ob_link_id=89").fetchone() == ("thru",)


def _mini_source(path, nodes, edges, graph):
    """A bare source db: nodes {id: (lon, lat)}, edges (id, a, b, lanes) as straight lines (one OSM
    way each, one-way), edge_graph rows (from, to)."""
    con = duckdb.connect(str(path))
    con.execute("INSTALL spatial; LOAD spatial; CREATE SCHEMA raw; CREATE SCHEMA driving")
    con.execute("CREATE TABLE raw.nodes(osm_id BIGINT, lat DOUBLE, lon DOUBLE, tags MAP(VARCHAR,VARCHAR))")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR,VARCHAR), refs BIGINT[])")
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY)")
    for n, (x, y) in nodes.items():
        con.execute(f"INSERT INTO driving.nodes VALUES ({n}, ST_Point({x}, {y}))")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, osm_id BIGINT, "
                "highway VARCHAR, name VARCHAR, lanes INTEGER, is_reverse BOOLEAN, oneway BOOLEAN, "
                "length_m FLOAT, maxspeed_kmh FLOAT, geometry GEOMETRY)")
    for e, a, b, n, *two in edges:               # (id, a, b, lanes[, osm way of a two-way road])
        (xa, ya), (xb, yb) = nodes[a], nodes[b]
        osm = two[0] if two else e
        con.execute(f"INSERT INTO raw.ways VALUES ({e}, MAP{{}}, [{a},{b}])")
        con.execute(f"INSERT INTO driving.edges VALUES ({e},{a},{b},{osm},'primary',NULL,{n},{'true' if two and a > b else 'false'},"
                    f"{'false' if two else 'true'},100,50,ST_GeomFromText('LINESTRING({xa} {ya},{xb} {yb})'))")
    con.execute("CREATE TABLE driving.edge_graph(from_edge BIGINT, to_edge BIGINT, via_edge BIGINT, cost DOUBLE)")
    for a, b in graph:
        con.execute(f"INSERT INTO driving.edge_graph VALUES ({a},{b},{b},1.0)")
    con.close()


def _mini_movements(tmp_path, nodes, edges, graph):
    src, out = tmp_path / "mini.duckdb", tmp_path / "mini_gmns.duckdb"
    _mini_source(src, nodes, edges, graph)
    to_gmns(str(src), str(out))
    return {(r[0], r[1]): r[2:] for r in duckdb.connect(str(out)).execute(
        "SELECT ib_link_id, ob_link_id, type, start_ib_lane, end_ib_lane, start_ob_lane, end_ob_lane "
        "FROM gmns_driving.movement").fetchall()}


def test_merge_lanes_follow_osm2gmns():
    from duckosm.gmns import _merge_lanes
    assert _merge_lanes([2, 1], 3) == [((0, 1), (0, 1)), ((0, 0), (2, 2))]   # main left, ramp into lane 3
    assert _merge_lanes([1, 1], 2) == [((0, 0), (0, 0)), ((0, 0), (1, 1))]
    assert _merge_lanes([2, 2], 2) == [((0, 1), (0, 1)), ((0, 1), (0, 1))]   # 2 into 2: both all lanes


def test_merge_stacks_the_joining_roads_and_is_typed_merge(tmp_path):
    """A 2-lane road (11) and a 1-lane ramp from the right (12, joining at 16 degrees) merge into a
    3-lane road (13): the road takes lanes 1-2, the ramp lane 3 (both had gone into lane 1), and
    both movements are typed 'merge'."""
    mv = _mini_movements(tmp_path, {1: (18.06, 59.32), 3: (18.06, 59.3185), 2: (18.07, 59.32), 4: (18.08, 59.32)},
                         [(11, 1, 2, 2), (12, 3, 2, 1), (13, 2, 4, 3)], [(11, 13), (12, 13)])
    assert mv[(11, 13)] == ("merge", 1, 2, 1, 2)
    assert mv[(12, 13)] == ("merge", 1, 1, 3, 3)


def test_fork_is_typed_diverge(tmp_path):
    """One road (11) splits into a straight road (12) and a slight right branch (13, 16 degrees):
    both movements 'diverge'. A crossroads with a 90-degree turn stays a junction."""
    nodes = {1: (18.06, 59.32), 2: (18.07, 59.32), 4: (18.08, 59.32), 5: (18.08, 59.3185), 6: (18.07, 59.31)}
    mv = _mini_movements(tmp_path, nodes, [(11, 1, 2, 2), (12, 2, 4, 1), (13, 2, 5, 1)], [(11, 12), (11, 13)])
    assert mv[(11, 12)][0] == "diverge" and mv[(11, 13)][0] == "diverge"
    cross = tmp_path / "cross"
    cross.mkdir()
    mv = _mini_movements(cross, nodes, [(11, 1, 2, 2), (12, 2, 4, 1), (14, 2, 6, 1)], [(11, 12), (11, 14)])
    assert mv[(11, 12)][0] == "thru" and mv[(11, 14)][0] == "right"


def _gmns_of(tmp_path, nodes, edges, graph):
    src, out = tmp_path / "lc.duckdb", tmp_path / "lc_gmns.duckdb"
    _mini_source(src, nodes, edges, graph)
    to_gmns(str(src), str(out))
    con = duckdb.connect(str(out))
    con.execute("LOAD spatial;")
    return con


def _pt(con, sql):
    x, y = con.execute(sql).fetchone()
    return x * 111320 * 0.5101, y * 111320                    # metres near 59.32 N (cos = 0.51)


def test_lane_connector_joins_lane_ends(tmp_path):
    """docs/design/gmns_lane_connectors.md: a turn's connector starts exactly at the inbound lane's
    end and ends at the outbound lane's start (base network: A's right turn into B)."""
    con = _gmns(tmp_path)
    c = con.execute(f"SELECT from_lane_id, to_lane_id FROM gmns_driving.lane_connector "
                    f"WHERE from_lane_id = '{A}_2' AND to_lane_id = '{B}_1'").fetchone()
    assert c is not None
    gap = con.execute(f"""SELECT ST_Distance(ST_StartPoint(c.geom), ST_EndPoint(a.geom)) + ST_Distance(ST_EndPoint(c.geom), ST_StartPoint(b.geom))
        FROM gmns_driving.lane_connector c, gmns_driving.lane a, gmns_driving.lane b
        WHERE c.from_lane_id = '{A}_2' AND c.to_lane_id = '{B}_1' AND a.lane_id = '{A}_2' AND b.lane_id = '{B}_1'""").fetchone()[0]
    assert gap < 1e-6


def test_a_lane_keeps_its_full_line_before_the_cut(tmp_path):
    """geom_full: each lane's line to its nodes, before it is cut back for the connectors (a reader drawing the lanes without connectors
    takes it, 2026-10-10): never shorter than geom, and longer where the lane ends inside a junction (A's lane 2, cut for its right turn)."""
    con = _gmns(tmp_path)
    shorter = con.execute("SELECT count(*) FROM gmns_driving.lane WHERE ST_Length(geom_full) < ST_Length(geom) - 1e-9").fetchone()[0]
    full, cut = con.execute(f"SELECT ST_Length(geom_full), ST_Length(geom) FROM gmns_driving.lane WHERE lane_id = '{A}_2'").fetchone()
    assert shorter == 0 and full > cut


def test_one_way_into_two_way_gets_an_s_curve(tmp_path):
    """A one-way lane (centred) going on into a two-way road (lanes 1.625 m to the side): no jump at
    the node; both lanes stop short and a connector shifts across (Avenue de la Costa)."""
    nodes = {1: (18.06, 59.32), 2: (18.07, 59.32), 3: (18.08, 59.32)}
    con = _gmns_of(tmp_path, nodes, [(11, 1, 2, 1), (12, 2, 3, 1, 500), (13, 3, 2, 1, 500)], [(11, 12)])
    row = con.execute("SELECT ST_Length_Spheroid(ST_FlipCoordinates(geom)) FROM gmns_driving.lane_connector WHERE from_lane_id = '11_1' "
                      "AND to_lane_id = '12_1'").fetchone()
    assert row is not None and 3.0 < row[0] < 6.0               # a short S-curve, not a 1.6 m step
    y0 = _pt(con, "SELECT ST_X(ST_StartPoint(geom)), ST_Y(ST_StartPoint(geom)) FROM gmns_driving.lane_connector WHERE from_lane_id = '11_1'")[1]
    y1 = _pt(con, "SELECT ST_X(ST_EndPoint(geom)), ST_Y(ST_EndPoint(geom)) FROM gmns_driving.lane_connector WHERE from_lane_id = '11_1'")[1]
    assert abs((y0 - y1) - 1.625) < 0.1                         # it shifts the lane 1.625 m to the right


def test_straight_road_needs_no_connector(tmp_path):
    nodes = {1: (18.06, 59.32), 2: (18.07, 59.32), 3: (18.08, 59.32)}
    con = _gmns_of(tmp_path, nodes, [(11, 1, 2, 1), (12, 2, 3, 1)], [(11, 12)])
    assert con.execute("SELECT count(*) FROM gmns_driving.lane_connector").fetchone()[0] == 0
    end = con.execute("SELECT ST_X(ST_EndPoint(geom)) FROM gmns_driving.lane WHERE lane_id = '11_1'").fetchone()[0]
    assert abs(end - 18.07) < 1e-7                              # not shortened


def test_lanes_stop_at_a_junction(tmp_path):
    """At a junction (3 neighbours) a lane stops where it leaves the other roads' lanes, not at the node:
    lane 2 of 11 (the south side) runs into the road leaving south (14, 6.5 m wide); lane 1 doesn't."""
    nodes = {1: (18.06, 59.32), 2: (18.07, 59.32), 4: (18.08, 59.32), 6: (18.07, 59.31)}
    con = _gmns_of(tmp_path, nodes, [(11, 1, 2, 2), (12, 2, 4, 2), (14, 2, 6, 2)], [(11, 12), (11, 14)])
    end = dict(con.execute("SELECT lane_id, ST_X(ST_EndPoint(geom)) FROM gmns_driving.lane WHERE link_id = 11").fetchall())
    assert (18.07 - end["11_2"]) * 111320 * 0.5101 > 2.0        # stops before the crossing road


def test_fork_lanes_unit():
    """Option B: the main exit keeps all its lanes, a branch shares the lanes on its side."""
    from duckosm.gmns import _fork_lanes
    # 2 lanes; a 1-lane branch on the left (index 0), the road going on (index 1, 2 lanes)
    assert _fork_lanes(2, [1, 2], 1) == [((0, 0), (0, 0)), ((0, 1), (0, 1))]
    # a branch on the right: it shares the right lane
    assert _fork_lanes(2, [2, 1], 0) == [((0, 1), (0, 1)), ((1, 1), (0, 0))]


def test_main_road_keeps_its_lanes_at_a_fork(tmp_path):
    """Boulevard du Larvotto (Kaveh): a 2-lane road going on under its own name and a 1-lane unnamed
    road branching off to the left at a shallow angle. Both lanes go on; lane 1 may also branch off
    (osm2gmns had sent lane 1 into the branch only)."""
    src, out = tmp_path / "fk.duckdb", tmp_path / "fk_gmns.duckdb"
    nodes = {1: (18.06, 59.32), 2: (18.07, 59.32), 4: (18.08, 59.32), 5: (18.08, 59.3215)}
    _mini_source(src, nodes, [(11, 1, 2, 2), (12, 2, 4, 2), (13, 2, 5, 1)], [(11, 12), (11, 13)])
    c = duckdb.connect(str(src))
    c.execute("UPDATE driving.edges SET name = 'Main' WHERE edge_id IN (11, 12)")
    c.close()
    to_gmns(str(src), str(out))
    mv = {(a, b): r for a, b, *r in duckdb.connect(str(out)).execute(
        "SELECT ib_link_id, ob_link_id, type, start_ib_lane, end_ib_lane, start_ob_lane, end_ob_lane "
        "FROM gmns_driving.movement").fetchall()}
    assert mv[(11, 12)] == ["diverge", 1, 2, 1, 2]
    assert mv[(11, 13)] == ["diverge", 1, 1, 1, 1]


def test_on_a_roundabout_the_ring_goes_on_at_a_fork(tmp_path):
    """A 2-lane roundabout piece forks into the ring's next piece (2 lanes, curving 50 degrees away) and a straighter 1-lane named exit (2026-10-10,
    Monaco 1174006399: the exit had been the road that goes on and the ring got lane 1 only, its lane 2 then deleted as unfed). The ring keeps both
    lanes; the exit (right of it) shares the outer lane."""
    src, out = tmp_path / "rb.duckdb", tmp_path / "rb_gmns.duckdb"
    nodes = {1: (18.06, 59.32), 2: (18.07, 59.32), 4: (18.0764, 59.3264), 5: (18.08, 59.3195)}
    _mini_source(src, nodes, [(11, 1, 2, 2), (12, 2, 4, 2), (13, 2, 5, 1)], [(11, 12), (11, 13)])
    c = duckdb.connect(str(src))
    c.execute("ALTER TABLE driving.edges ADD COLUMN IF NOT EXISTS junction VARCHAR")
    c.execute("UPDATE driving.edges SET junction = 'roundabout' WHERE edge_id IN (11, 12)")
    c.execute("UPDATE driving.edges SET name = 'Exit' WHERE edge_id = 13")
    c.close()
    to_gmns(str(src), str(out))
    con = duckdb.connect(str(out))
    mv = {(a, b): r for a, b, *r in con.execute("SELECT ib_link_id, ob_link_id, start_ib_lane, end_ib_lane, start_ob_lane, end_ob_lane "
                                                "FROM gmns_driving.movement").fetchall()}
    assert mv[(11, 12)] == [1, 2, 1, 2] and mv[(11, 13)] == [2, 2, 1, 1]
    assert con.execute("SELECT count(*) FROM gmns_driving.lane WHERE link_id = 12").fetchone()[0] == 2


def _mv_of(tmp_path, nodes, edges, graph, names=None, tags=None):
    src, out = tmp_path / "jn.duckdb", tmp_path / "jn_gmns.duckdb"
    _mini_source(src, nodes, edges, graph)
    c = duckdb.connect(str(src))
    for e, nm in (names or {}).items():
        c.execute(f"UPDATE driving.edges SET name = '{nm}' WHERE edge_id = {e}")
    for e, t in (tags or {}).items():
        c.execute(f"UPDATE raw.ways SET tags = MAP{{{', '.join(f'{k!r}:{v!r}' for k, v in t.items())}}} WHERE osm_id = {e}")
    c.close()
    to_gmns(str(src), str(out))
    return {(a, b): tuple(r) for a, b, *r in duckdb.connect(str(out)).execute(
        "SELECT ib_link_id, ob_link_id, start_ib_lane, end_ib_lane, start_ob_lane, end_ob_lane FROM gmns_driving.movement").fetchall()}


def test_road_going_on_keeps_its_lanes_at_a_junction(tmp_path):
    """3358160335623944038 (Kaveh): a 2-lane road X goes on straight past a side road. Straight on
    keeps both lanes (osm2gmns gave the right lane to the right turn only), the right turn shares
    lane 2, and a single right turn from the side road enters the rightmost lane (osm2gmns: lane 1),
    so lane 2 of the road ahead has a way in."""
    nodes = {1: (18.06, 59.32), 2: (18.07, 59.32), 4: (18.08, 59.32), 6: (18.0702, 59.31), 7: (18.0698, 59.31)}
    mv = _mv_of(tmp_path, nodes, [(11, 1, 2, 2), (12, 2, 4, 2), (14, 2, 6, 1), (15, 7, 2, 1)],
                [(11, 12), (11, 14), (15, 12)], names={11: "X", 12: "X"})
    assert mv[(11, 12)] == (1, 2, 1, 2)          # straight on from both lanes
    assert mv[(11, 14)] == (2, 2, 1, 1)          # the right turn shares the right lane
    assert mv[(15, 12)] == (1, 1, 2, 2)          # a lone right turn enters the right lane


def test_arrows_without_their_exit_go_ahead(tmp_path):
    """Avenue de Fontvieille: turn:lanes 'left|left|' where the way ends at a fork with no left exit
    (straight on, a slight right branch). The left lanes go ahead with the unmarked lane (they had no
    way on), the branch shares the right lane."""
    nodes = {1: (18.06, 59.32), 2: (18.07, 59.32), 4: (18.08, 59.32), 5: (18.08, 59.3185)}
    mv = _mv_of(tmp_path, nodes, [(11, 1, 2, 3), (12, 2, 4, 2), (13, 2, 5, 1)], [(11, 12), (11, 13)],
                tags={11: {"turn:lanes": "left|left|"}})
    assert mv[(11, 12)][:2] == (1, 2)            # lanes 1-2 (both left arrows) go on
    assert mv[(11, 13)][:2] == (3, 3)


def _way_source(path, nodes, edges):
    """A bare source db where edges share OSM ways: edges (id, a, b, lanes, osm_id, oneway)."""
    con = duckdb.connect(str(path))
    con.execute("INSTALL spatial; LOAD spatial; CREATE SCHEMA raw; CREATE SCHEMA driving")
    con.execute("CREATE TABLE raw.nodes(osm_id BIGINT, lat DOUBLE, lon DOUBLE, tags MAP(VARCHAR,VARCHAR))")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR,VARCHAR), refs BIGINT[])")
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY)")
    for n, (x, y) in nodes.items():
        con.execute(f"INSERT INTO driving.nodes VALUES ({n}, ST_Point({x}, {y}))")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, osm_id BIGINT, "
                "highway VARCHAR, name VARCHAR, lanes INTEGER, is_reverse BOOLEAN, oneway BOOLEAN, "
                "length_m FLOAT, maxspeed_kmh FLOAT, geometry GEOMETRY)")
    for osm in {e[4] for e in edges}:
        con.execute(f"INSERT INTO raw.ways VALUES ({osm}, MAP{{}}, [])")
    for e, a, b, n, osm, ow in edges:
        (xa, ya), (xb, yb) = nodes[a], nodes[b]
        con.execute(f"INSERT INTO driving.edges VALUES ({e},{a},{b},{osm},'primary','Main',{n},false,{str(ow).lower()},"
                    f"100,50,ST_GeomFromText('LINESTRING({xa} {ya},{xb} {yb})'))")
    con.execute("CREATE TABLE driving.edge_graph(from_edge BIGINT, to_edge BIGINT, via_edge BIGINT, cost DOUBLE)")
    for e, a, b, *_ in edges:
        for f, c, d, *_ in edges:
            if b == c and e != f:
                con.execute(f"INSERT INTO driving.edge_graph VALUES ({e},{f},{f},1.0)")
    con.close()


def _runs_gmns(tmp_path, nodes, edges):
    src, out = tmp_path / "runs.duckdb", tmp_path / "runs_gmns.duckdb"
    _way_source(src, nodes, edges)
    to_gmns(str(src), str(out))
    con = duckdb.connect(str(out))
    con.execute("LOAD spatial;")
    return con


def _m(con, sql):
    """A distance query's result, degrees -> metres near 59.32 N isn't needed: use ST_Distance_Sphere."""
    return con.execute(sql).fetchone()[0]


def test_lane_is_one_curve_across_a_bend(tmp_path):
    """docs/design/gmns_lane_runs.md: a way in two pieces with a 35-degree bend at node 2 (no other
    road there). Offset per piece, the lanes left a wedge at the node; offset once per run, lane 11_1
    ends exactly where 12_1 starts, and no connector is made."""
    nodes = {1: (18.06, 59.32), 2: (18.07, 59.32), 3: (18.078, 59.3235)}
    con = _runs_gmns(tmp_path, nodes, [(11, 1, 2, 2, 500, True), (12, 2, 3, 2, 500, True)])
    for k in (1, 2):
        gap = _m(con, f"SELECT ST_Distance_Sphere(ST_EndPoint(a.geom), ST_StartPoint(b.geom)) FROM gmns_driving.lane a, "
                      f"gmns_driving.lane b WHERE a.lane_id='11_{k}' AND b.lane_id='12_{k}'")
        assert gap < 0.05, gap
    assert _m(con, "SELECT count(*) FROM gmns_driving.lane_connector") == 0


def test_short_piece_inherits_the_runs_pairing(tmp_path):
    """A 2-lane one-way road in two pieces (10 m, then 4 m) with its opposite carriageway 4 m to the
    left: the 4 m piece alone was too short to be paired and its lanes jumped back to centred. Paired
    per run, lane 1 is at the same offset in both pieces."""
    y, d = 59.32, 4.0 / 111320
    nodes = {1: (18.0600, y), 2: (18.00018 + 18.0600, y), 3: (18.00025 + 18.0600, y), 5: (18.0603, y + d), 6: (18.0600, y + d)}
    con = _runs_gmns(tmp_path, nodes, [(11, 1, 2, 2, 500, True), (12, 2, 3, 2, 500, True), (21, 5, 6, 2, 600, True)])
    off = lambda lid: _m(con, f"SELECT (ST_Y(ST_LineInterpolatePoint(l.geom, 0.5)) - {y}) * 111320 FROM gmns_driving.lane l WHERE lane_id='{lid}'")
    assert abs(off("11_1") - off("12_1")) < 0.05 and abs(off("11_2") - off("12_2")) < 0.05
    assert _m(con, "SELECT count(*) FROM gmns_driving.lane_connector WHERE from_lane_id LIKE '11_%' AND to_lane_id LIKE '12_%'") == 0


def test_lane_going_on_through_a_junction_is_not_cut(tmp_path):
    """Way 500 goes straight through node 2, where a 2-lane side road leaves south. Its lanes used to
    be cut inside the side road's lanes and joined back by a connector; now they run through intact."""
    nodes = {1: (18.06, 59.32), 2: (18.07, 59.32), 3: (18.08, 59.32), 6: (18.07, 59.31)}
    con = _runs_gmns(tmp_path, nodes, [(11, 1, 2, 2, 500, True), (12, 2, 3, 2, 500, True), (14, 2, 6, 2, 700, True)])
    end_x = _m(con, "SELECT ST_X(ST_EndPoint(geom)) FROM gmns_driving.lane WHERE lane_id = '11_2'")
    assert abs(end_x - 18.07) < 1e-7                              # not trimmed at the junction
    assert _m(con, "SELECT count(*) FROM gmns_driving.lane_connector WHERE from_lane_id LIKE '11_%' AND to_lane_id LIKE '12_%'") == 0
    assert _m(con, "SELECT count(*) FROM gmns_driving.lane_connector WHERE from_lane_id = '11_2' AND to_lane_id = '14_2'") == 1


def test_trimming_keeps_at_least_two_metres(tmp_path):
    """A 4 m piece of its own way between two junctions (a 1-lane road into a 2-lane piece into a
    1-lane road, side roads at both nodes) used to be cut to a stub; it keeps at least 2 m."""
    d4 = 4.0 / 111320 / 0.5101
    nodes = {1: (18.06, 59.32), 2: (18.07, 59.32), 3: (18.07 + d4, 59.32), 4: (18.08, 59.32), 6: (18.07, 59.31), 7: (18.07 + d4, 59.31)}
    con = _runs_gmns(tmp_path, nodes, [(11, 1, 2, 1, 500, True), (12, 2, 3, 2, 600, True), (13, 3, 4, 1, 700, True),
                                        (16, 2, 6, 2, 800, True), (17, 3, 7, 2, 900, True)])
    for k in (1, 2):
        assert _m(con, f"SELECT ST_Length_Spheroid(ST_FlipCoordinates(geom)) FROM gmns_driving.lane WHERE lane_id='12_{k}'") >= 1.95


def test_a_roundabout_ring_closes(tmp_path):
    """A roundabout drawn as 4 one-way ways around a square: the run is a ring, so the closing joint
    (last piece into the first) meets like the others, with no connector anywhere on the ring."""
    src, out = tmp_path / "ring.duckdb", tmp_path / "ring_gmns.duckdb"
    d = 12 / 111320
    nodes = {1: (18.06, 59.32), 2: (18.06 + d * 2, 59.32), 3: (18.06 + d * 2, 59.32 + d), 4: (18.06, 59.32 + d)}
    _way_source(src, nodes, [(11, 1, 2, 2, 501, True), (12, 2, 3, 2, 502, True), (13, 3, 4, 2, 503, True), (14, 4, 1, 2, 504, True)])
    c = duckdb.connect(str(src))
    c.execute("UPDATE raw.ways SET tags = MAP{'junction':'roundabout'}")
    c.close()
    to_gmns(str(src), str(out))
    con = duckdb.connect(str(out))
    con.execute("LOAD spatial;")
    for a, b in ((11, 12), (12, 13), (13, 14), (14, 11)):
        gap = con.execute(f"SELECT max(ST_Distance_Sphere(ST_EndPoint(x.geom), ST_StartPoint(y.geom))) FROM gmns_driving.lane x, "
                          f"gmns_driving.lane y WHERE x.link_id={a} AND y.link_id={b} AND x.lane_num = y.lane_num").fetchone()[0]
        assert gap < 0.05, (a, b, gap)
    assert con.execute("SELECT count(*) FROM gmns_driving.lane_connector").fetchone()[0] == 0


def test_a_uturn_at_a_road_end_is_a_half_circle_as_wide_as_its_lanes(tmp_path):
    """docs/design/gmns_uturn_arc.md: a dead-end road's two lane ends are one lane width apart; the U-turn joins them with a half-circle (every point
    of its centre line chord/2 from the middle of the two ends) and is as wide as the chord, not narrowed to 1.8 x the tightest radius."""
    nodes = {1: (18.06, 59.32), 2: (18.07, 59.32)}
    con = _gmns_of(tmp_path, nodes, [(11, 1, 2, 1, 500), (12, 2, 1, 1, 500)], [(11, 12)])
    w, wkt = con.execute("SELECT width, ST_AsText(geom) FROM gmns_driving.lane_connector WHERE from_lane_id = '11_1' AND to_lane_id = '12_1'").fetchone()
    pts = [tuple(float(v) for v in p.split()) for p in wkt[wkt.index("(") + 1:-1].split(", ")]
    m = lambda p: (p[0] * 111320 * 0.5101, p[1] * 111320)                # noqa: E731  metres near 59.32 N
    (x0, y0), (x1, y1) = m(pts[0]), m(pts[-1])
    chord, mid = ((x1 - x0) ** 2 + (y1 - y0) ** 2) ** 0.5, ((x0 + x1) / 2, (y0 + y1) / 2)
    assert chord == pytest.approx(3.25, abs=0.05) and w == pytest.approx(chord, abs=0.02)      # one lane apart, as wide as the lanes
    assert all(abs(((m(p)[0] - mid[0]) ** 2 + (m(p)[1] - mid[1]) ** 2) ** 0.5 - chord / 2) < 0.1 for p in pts)


def test_smooth_ring_puts_a_circle_through_a_coarse_roundabout_and_leaves_other_rings_alone():
    """docs/design/gmns_roundabout_arc.md: the vertices of a ring that lies on a circle are kept and points on the circle are added (3 degrees at most); a square, an oval or an open line is not changed."""
    import math

    from duckosm.gmns import _smooth_ring

    n, r = 8, 10.0
    ring = [(r * math.cos(2 * math.pi * k / n), r * math.sin(2 * math.pi * k / n)) for k in range(n)]
    ring.append(ring[0])
    out = _smooth_ring(ring)
    assert set(ring) <= set(out) and len(out) >= 120 and out[0] == out[-1]                   # the original vertices stay, the arcs are dense
    assert max(abs(math.hypot(x, y) - r) for x, y in out) < 1e-9
    assert max(math.degrees(abs(math.atan2(b[1], b[0]) - math.atan2(a[1], a[0]) + math.pi) % (2 * math.pi) - math.pi) for a, b in zip(out, out[1:])) < 3.0001
    square = [(0, 0), (10, 0), (10, 10), (0, 10), (0, 0)]
    oval = [(15 * math.cos(2 * math.pi * k / 12), 8 * math.sin(2 * math.pi * k / 12)) for k in range(12)]
    oval.append(oval[0])
    open_line = [(0, 0), (5, 1), (10, 3), (15, 6), (20, 10)]
    assert _smooth_ring(square) == square and _smooth_ring(oval) == oval and _smooth_ring(open_line) == open_line


def test_a_roundabout_of_a_few_ways_has_round_lanes(tmp_path):
    """The lanes of a roundabout drawn as 8 ways around a 10 m circle are on circles (the radius of a lane varies by less than 5 cm), not on an octagon."""
    import math

    src, out = tmp_path / "round.duckdb", tmp_path / "round_gmns.duckdb"
    kx = 111320 * math.cos(math.radians(59.32))
    nodes = {k + 1: (18.06 + 10 * math.cos(2 * math.pi * k / 8) / kx, 59.32 + 10 * math.sin(2 * math.pi * k / 8) / 111320) for k in range(8)}
    _way_source(src, nodes, [(11 + k, k + 1, (k + 1) % 8 + 1, 2, 501 + k, True) for k in range(8)])
    c = duckdb.connect(str(src))
    c.execute("UPDATE raw.ways SET tags = MAP{'junction':'roundabout'}")
    c.close()
    to_gmns(str(src), str(out))
    con = duckdb.connect(str(out))
    con.execute("LOAD spatial;")
    for lane in ("11_1", "11_2", "14_1"):
        wkt = con.execute(f"SELECT ST_AsText(geom) FROM gmns_driving.lane WHERE lane_id='{lane}'").fetchone()[0]
        pts = [tuple(float(v) for v in p.split()) for p in wkt[wkt.index("(") + 1:-1].split(", ")]
        rad = [math.hypot((x - 18.06) * kx, (y - 59.32) * 111320) for x, y in pts]
        assert len(pts) > 6 and max(rad) - min(rad) < 0.05, (lane, len(pts), max(rad) - min(rad))


def test_the_branches_of_a_fork_carry_their_own_turn_letter(tmp_path):
    """docs/design/gmns_fork_letters.md: a road (11) splits into a straight branch (12) and a branch 16 degrees to the right (13): both are `diverge`, and the codes say T and R. Two branches within 8 degrees of each other are both T."""
    nodes = {1: (18.06, 59.32), 2: (18.07, 59.32), 4: (18.08, 59.32), 5: (18.08, 59.3185), 6: (18.08, 59.3197)}
    con = _gmns_of(tmp_path, nodes, [(11, 1, 2, 2), (12, 2, 4, 1), (13, 2, 5, 1)], [(11, 12), (11, 13)])
    codes = dict(con.execute("SELECT ob_link_id, mvmt_code FROM gmns_driving.movement WHERE ib_link_id = 11").fetchall())
    assert codes == {12: "EBT", 13: "EBR"}
    left = tmp_path / "left"
    left.mkdir()
    con = _gmns_of(left, {1: (18.06, 59.32), 2: (18.07, 59.32), 4: (18.08, 59.32), 5: (18.08, 59.3215)}, [(11, 1, 2, 2), (12, 2, 4, 1), (13, 2, 5, 1)], [(11, 12), (11, 13)])
    assert dict(con.execute("SELECT ob_link_id, mvmt_code FROM gmns_driving.movement WHERE ib_link_id = 11").fetchall()) == {12: "EBT", 13: "EBL"}
    flat = tmp_path / "flat"
    flat.mkdir()
    con = _gmns_of(flat, {1: (18.06, 59.32), 2: (18.07, 59.32), 4: (18.08, 59.32), 6: (18.08, 59.3197)}, [(11, 1, 2, 2), (12, 2, 4, 1), (13, 2, 6, 1)], [(11, 12), (11, 13)])
    assert set(dict(con.execute("SELECT ob_link_id, mvmt_code FROM gmns_driving.movement WHERE ib_link_id = 11").fetchall()).values()) == {"EBT"}      # 3 degrees apart: both straight


def test_a_fork_branch_keeps_only_the_lanes_the_fork_feeds():
    """docs/design/gmns_fork_lanes.md: branch 12 has 2 lanes but only lane 2 is fed (by movement 11-12), and it goes on into link 14 (2 lanes): lane 1 of 12 goes, link 12 has 1 lane, and the movement 12-14 is cut to lane 2 -> lane 2.
    A branch fed on both lanes, or one that goes on into a link with fewer lanes, is left alone."""
    from duckosm.gmns import _fork_branch_lanes

    con = duckdb.connect(":memory:")
    con.execute("CREATE SCHEMA gmns_driving")
    con.execute("CREATE TEMP TABLE _extra_lane(link_id BIGINT, lane_num INTEGER)")
    con.execute("CREATE TABLE gmns_driving.lane(lane_id VARCHAR, link_id BIGINT, lane_num INTEGER)")
    con.execute("CREATE TABLE gmns_driving.link(link_id BIGINT, lanes INTEGER)")
    con.execute("""CREATE TABLE gmns_driving.movement(mvmt_id VARCHAR, ib_link_id BIGINT, ob_link_id BIGINT, start_ib_lane INTEGER, end_ib_lane INTEGER,
                                                    start_ob_lane INTEGER, end_ob_lane INTEGER, type VARCHAR)""")
    for lk in (11, 12, 13, 14, 15, 16, 17, 18):
        con.execute("INSERT INTO gmns_driving.link VALUES (?, 2)", [lk])
        for n in (1, 2):
            con.execute("INSERT INTO gmns_driving.lane VALUES (?, ?, ?)", [f"{lk}_{n}", lk, n])
    mv = [("11-12", 11, 12, 2, 2, 2, 2, "diverge"), ("11-13", 11, 13, 1, 1, 1, 1, "diverge"),      # 12 fed on lane 2 only, 13 on lane 1 only
          ("12-14", 12, 14, 1, 2, 1, 2, "thru"), ("13-15", 13, 15, 1, 2, 1, 2, "thru"),
          ("21-16", 21, 16, 1, 2, 1, 2, "diverge"), ("21-17", 21, 17, 1, 2, 1, 2, "diverge"), ("16-18", 16, 18, 1, 2, 1, 2, "thru")]   # 16 is fed on both lanes
    con.executemany("INSERT INTO gmns_driving.movement VALUES (?, ?, ?, ?, ?, ?, ?, ?)", mv)
    gone = _fork_branch_lanes(con, "gmns_driving")
    assert gone == {12: [1], 13: [2]}, gone
    assert con.execute("SELECT lane_num FROM gmns_driving.lane WHERE link_id = 12").fetchall() == [(2,)]
    assert con.execute("SELECT lanes FROM gmns_driving.link WHERE link_id IN (12, 13) ORDER BY link_id").fetchall() == [(1,), (1,)]
    assert con.execute("SELECT start_ib_lane, end_ib_lane, start_ob_lane, end_ob_lane FROM gmns_driving.movement WHERE mvmt_id = '12-14'").fetchone() == (2, 2, 2, 2)
    assert con.execute("SELECT start_ib_lane, end_ib_lane, start_ob_lane, end_ob_lane FROM gmns_driving.movement WHERE mvmt_id = '13-15'").fetchone() == (1, 1, 1, 1)
    assert con.execute("SELECT count(*) FROM gmns_driving.lane WHERE link_id = 16").fetchone()[0] == 2         # fed on both lanes: left alone


def test_bus_only_edges_are_one_lane_links(tmp_path):
    """docs/design/bus_only_edges.md: the bus-only edges of driving.private_edges are GMNS links of one lane. Boulevard Charles III
    (Monaco, way 1449981121): a one-way secondary, lanes=2, with a bus lane back (oneway:bus=no) shared with bikes
    (cycleway:left=share_busway): its forward link keeps 2 auto lanes, the reverse is one 'bus,bike' lane of 3.25 m on its own side.
    A bus-only road (no bikes) is one 'bus' lane, its node added though driving.nodes lacks it; neither gets a movement."""
    F, R, S = 778772381524539079, 458517452137650958, 77
    src, out = tmp_path / "src.duckdb", tmp_path / "out_gmns.duckdb"
    con = duckdb.connect(str(src))
    con.execute("INSTALL spatial; LOAD spatial; CREATE SCHEMA raw; CREATE SCHEMA driving")
    con.execute("CREATE TABLE raw.nodes(osm_id BIGINT, lat DOUBLE, lon DOUBLE, tags MAP(VARCHAR,VARCHAR))")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR,VARCHAR), refs BIGINT[])")
    con.execute("""INSERT INTO raw.ways VALUES (200, MAP{'highway':'secondary','oneway':'yes','lanes':'2','oneway:bus':'no',
        'oneway:bicycle':'no','cycleway:left':'share_busway','sidewalk':'left'}, [1,2]),
        (201, MAP{'highway':'service','access':'no','bus':'designated'}, [2,3])""")
    p = lambda w: f"ST_GeomFromText('{w}')"
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY)")
    con.execute(f"INSERT INTO driving.nodes VALUES (1,{p('POINT(18.06 59.32)')}),(2,{p('POINT(18.07 59.32)')})")   # 3 is on the bus road only
    cols = ("edge_id BIGINT, source BIGINT, target BIGINT, osm_id BIGINT, highway VARCHAR, name VARCHAR, lanes INTEGER, oneway BOOLEAN, "
            "access VARCHAR, is_reverse BOOLEAN, length_m FLOAT, maxspeed_kmh FLOAT, geometry GEOMETRY")
    con.execute(f"CREATE TABLE driving.edges({cols})")
    con.execute(f"CREATE TABLE driving.private_edges({cols})")
    con.execute(f"INSERT INTO driving.edges VALUES ({F},1,2,200,'secondary','Charles III',2,true,NULL,false,570,50,"
                f"{p('LINESTRING(18.06 59.32,18.07 59.32)')})")
    con.execute(f"""INSERT INTO driving.private_edges VALUES
        ({R},2,1,200,'secondary','Charles III',2,true,'bus',true,570,50,{p('LINESTRING(18.07 59.32,18.06 59.32)')}),
        ({S},2,3,201,'service',NULL,1,false,'bus',false,1110,30,{p('LINESTRING(18.07 59.32,18.07 59.31)')}),
        (78,1,2,202,'service',NULL,1,false,'private',false,570,30,{p('LINESTRING(18.06 59.32,18.07 59.32)')})""")
    con.execute("CREATE TABLE driving.edge_graph(from_edge BIGINT, to_edge BIGINT, via_edge BIGINT, cost DOUBLE)")
    con.close()
    to_gmns(str(src), str(out))
    con = duckdb.connect(str(out))
    con.execute("LOAD spatial;")
    q = lambda s: con.execute(s).fetchall()
    assert q("SELECT link_id, lanes, allowed_uses FROM gmns_driving.link ORDER BY link_id") == [
        (S, 1, "bus"), (R, 1, "bus,bike"), (F, 2, "auto")]                     # the way's lanes=2 is the forward's; no private road
    assert q("SELECT link_id, lane_num, allowed_uses, width, geom IS NOT NULL FROM gmns_driving.lane ORDER BY link_id, lane_num") == [
        (S, 1, "bus", 3.25, True), (R, 1, "bus,bike", 3.25, True), (F, 1, "auto", 3.25, True), (F, 2, "auto", 3.25, True)]
    north = dict(q("SELECT lane_id, ST_Y(ST_Centroid(geom)) > 59.32 FROM gmns_driving.lane WHERE link_id IN (" + f"{F}, {R})"))
    assert north == {f"{R}_1": True, f"{F}_1": False, f"{F}_2": False}      # the bus lane on its own side: left of the eastbound lanes
    assert q("SELECT node_id FROM gmns_driving.node ORDER BY node_id") == [(1,), (2,), (3,)]
    assert q("SELECT count(*) FROM gmns_driving.movement") == [(0,)]          # not in edge_graph: no turns made up


def test_bus_turns_come_from_edge_graph_and_leave_the_cars_alone(tmp_path):
    """docs/design/bus_only_edges.md, bus turns: the edge_graph builder adds the turns into and out of a bus-only edge
    (uses 'bus'), routing reads only the cars' rows, GMNS makes them bus movements with lanes; the cars' movements are
    those of the same graph without the bus rows. Way 200 (1->2) is one-way with a bus lane back (2->1, bikes too);
    way 203 (2<->4) is two-way; 201 (2->3) is a bus-only road."""
    from duckosm.processors.edge_graph import EdgeGraphBuilder, routed_graph
    from duckosm.gmns import to_gmns
    F, R, S, G, Gr = 11, 12, 13, 14, 15
    src = tmp_path / "src.duckdb"
    con = duckdb.connect(str(src))
    con.execute("INSTALL spatial; LOAD spatial; CREATE SCHEMA raw; CREATE SCHEMA driving")
    con.execute("CREATE TABLE raw.nodes(osm_id BIGINT, lat DOUBLE, lon DOUBLE, tags MAP(VARCHAR,VARCHAR))")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR,VARCHAR), refs BIGINT[])")
    con.execute("""INSERT INTO raw.ways VALUES (200, MAP{'highway':'secondary','oneway':'yes','oneway:bus':'no','oneway:bicycle':'no'}, [1,2]),
        (201, MAP{'highway':'service','access':'no','bus':'designated'}, [2,3]), (203, MAP{'highway':'secondary'}, [2,4])""")
    p = lambda w: f"ST_GeomFromText('{w}')"
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY)")
    con.execute(f"INSERT INTO driving.nodes VALUES (1,{p('POINT(18.06 59.32)')}),(2,{p('POINT(18.07 59.32)')}),(4,{p('POINT(18.08 59.32)')})")
    cols = ("edge_id BIGINT, source BIGINT, target BIGINT, osm_id BIGINT, highway VARCHAR, name VARCHAR, lanes INTEGER, oneway BOOLEAN, "
            "access VARCHAR, is_reverse BOOLEAN, length_m FLOAT, maxspeed_kmh FLOAT, cost_s FLOAT, geometry GEOMETRY")
    con.execute(f"CREATE TABLE driving.edges({cols})")
    con.execute(f"CREATE TABLE driving.private_edges({cols})")
    con.execute(f"""INSERT INTO driving.edges VALUES
        ({F},1,2,200,'secondary','A',1,true,NULL,false,570,50,41,{p('LINESTRING(18.06 59.32,18.07 59.32)')}),
        ({G},2,4,203,'secondary','B',1,false,NULL,false,570,50,41,{p('LINESTRING(18.07 59.32,18.08 59.32)')}),
        ({Gr},4,2,203,'secondary','B',1,false,NULL,true,570,50,41,{p('LINESTRING(18.08 59.32,18.07 59.32)')})""")
    con.execute(f"""INSERT INTO driving.private_edges VALUES
        ({R},2,1,200,'secondary','A',1,true,'bus',true,570,50,41,{p('LINESTRING(18.07 59.32,18.06 59.32)')}),
        ({S},2,3,201,'service',NULL,1,false,'bus',false,1110,30,133,{p('LINESTRING(18.07 59.32,18.07 59.31)')})""")
    con.execute("USE driving")
    EdgeGraphBuilder(con, "driving").run()
    q = lambda s: con.execute(s).fetchall()
    assert set(q("SELECT from_edge, to_edge FROM edge_graph WHERE uses = 'car'")) == {(F, G), (G, Gr), (Gr, G)}
    assert set(q("SELECT from_edge, to_edge FROM edge_graph WHERE uses = 'bus'")) == {(F, R), (F, S), (Gr, R), (Gr, S), (R, F)}
    assert set(q(f"SELECT from_edge, to_edge FROM {routed_graph(con, 'driving')}")) == {(F, G), (G, Gr), (Gr, G)}
    con.close()
    nobus = tmp_path / "nobus.duckdb"
    import shutil
    shutil.copy(src, nobus)
    c2 = duckdb.connect(str(nobus))
    c2.execute("DELETE FROM driving.edge_graph WHERE uses <> 'car'")
    c2.close()
    to_gmns(str(src), str(tmp_path / "bus_gmns.duckdb"), modes=["driving"])
    to_gmns(str(nobus), str(tmp_path / "nobus_gmns.duckdb"), modes=["driving"])
    g = duckdb.connect(str(tmp_path / "bus_gmns.duckdb"))
    g.execute(f"ATTACH '{tmp_path / 'nobus_gmns.duckdb'}' AS n")
    mv = lambda where: g.execute(f"SELECT mvmt_id, type, allowed_uses, start_ib_lane, end_ib_lane, start_ob_lane, end_ob_lane "
                                 f"FROM gmns_driving.movement WHERE {where} ORDER BY 1").fetchall()
    assert mv("allowed_uses LIKE 'bus%'") == [               # F->R, a U-turn onto the way's other direction, has another way on: none
        (f"{F}-{S}", "right", "bus", 1, 1, 1, 1), (f"{R}-{F}", "uturn", "bus,bike", 1, 1, 1, 1),
        (f"{Gr}-{R}", "thru", "bus,bike", 1, 1, 1, 1), (f"{Gr}-{S}", "left", "bus", 1, 1, 1, 1)]
    assert mv("allowed_uses NOT LIKE 'bus%'") == g.execute(
        "SELECT mvmt_id, type, allowed_uses, start_ib_lane, end_ib_lane, start_ob_lane, end_ob_lane FROM n.gmns_driving.movement ORDER BY 1").fetchall()
    assert g.execute(f"SELECT count(*) FROM gmns_driving.lane_connector WHERE mvmt_id = '{Gr}-{S}'").fetchone()[0] == 1
    g.close()
    from duckosm.lane_routing import build_lane_graph
    build_lane_graph(str(tmp_path / "bus_gmns.duckdb"))         # the cars' lane graph: no bus turns
    g = duckdb.connect(str(tmp_path / "bus_gmns.duckdb"))
    assert not g.execute(f"SELECT * FROM lane_driving.lane_edges WHERE from_lane LIKE '{R}_%' OR to_lane LIKE '{S}_%'").fetchall()
    assert g.execute(f"SELECT count(*) FROM lane_driving.lane_edges WHERE from_lane = '{F}_1' AND to_lane = '{G}_1'").fetchone()[0] == 1
