"""Tests for duckosm.gmns.to_gmns — extract a built network to a standalone GMNS DuckDB.

Builds a tiny source db (with raw OSM tags, a signal, lane tags, parking) on disk, runs the
extractor, and checks the GMNS tables: link_id=edge_id, referential integrity, per-lane detail from
OSM tags, movement turn types with the immediate-reversal U-turn dropped, signals, and curb.
"""
import duckdb
import pytest

pytest.importorskip("pandas")
from duckosm.gmns import to_gmns, to_meso  # noqa: E402

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
    con = _gmns(tmp_path)
    # A->AR is the immediate reversal → excluded; A->B (turning north) kept and typed
    assert con.execute(
        f"SELECT count(*) FROM gmns_driving.movement WHERE ib_link_id={A} AND ob_link_id={AR}").fetchone()[0] == 0
    row = con.execute(
        f"SELECT type, ctrl_type FROM gmns_driving.movement WHERE ib_link_id={A} AND ob_link_id={B}").fetchone()
    assert row is not None and row[0] in ("left", "right")       # a real turn at the junction
    assert row[1] == "signal"                                    # node 2 is signalized


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
    con = duckdb.connect(str(src))
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
    out = tmp_path / "out_gmns.duckdb"
    to_gmns(str(src), str(out), modes=["driving", "cycling"])
    to_meso(str(out), modes=["driving", "cycling"])
    con = duckdb.connect(str(out))
    for m in ("driving", "cycling"):
        assert con.execute(
            f"SELECT count(*) FROM meso_{m}.meso_link WHERE meso_type='normal'").fetchone()[0] == 2


def test_to_csv_is_spec_clean(tmp_path):
    out_csv = tmp_path / "csv"
    con = _gmns(tmp_path, modes=["driving"], to_csv=str(out_csv))
    con.close()
    import csv
    with open(out_csv / "lane.csv") as f:
        cols = next(csv.reader(f))
    assert "geom" not in cols and "turn" not in cols            # non-spec columns dropped for CSV
    assert cols[:3] == ["lane_id", "link_id", "lane_num"]
