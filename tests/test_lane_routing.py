"""Tests for duckosm.lane_routing — the lane graph + lane-level router."""
import duckdb
import pytest

from duckosm.gmns import to_gmns
from duckosm.lane_routing import build_lane_graph, route_lanes

A, B, C, AR = 6141068311830699705, 3843102655846694531, 5555555555555555555, 1234567890123456789


def _gmns(tmp_path):
    """GMNS db: 2-lane primary A(1→2) with a turn choice to B(2→3) and C(2→4)."""
    src = tmp_path / "src.duckdb"
    con = duckdb.connect(str(src))
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA driving")
    p = lambda w: f"ST_GeomFromText('{w}')"
    con.execute("CREATE TABLE driving.nodes(node_id BIGINT, geom GEOMETRY)")
    con.execute(f"INSERT INTO driving.nodes VALUES (1,{p('POINT(18.06 59.32)')}),(2,{p('POINT(18.07 59.32)')}),"
                f"(3,{p('POINT(18.07 59.31)')}),(4,{p('POINT(18.08 59.32)')})")
    con.execute("CREATE TABLE driving.edges(edge_id BIGINT, source BIGINT, target BIGINT, osm_id BIGINT, "
                "highway VARCHAR, name VARCHAR, lanes INTEGER, is_reverse BOOLEAN, length_m FLOAT, "
                "maxspeed_kmh FLOAT, geometry GEOMETRY)")
    con.execute(f"""INSERT INTO driving.edges VALUES
        ({A},1,2,100,'primary','Main',2,false,80,50,{p('LINESTRING(18.06 59.32,18.07 59.32)')}),
        ({B},2,3,101,'residential',NULL,1,false,110,30,{p('LINESTRING(18.07 59.32,18.07 59.31)')}),
        ({C},2,4,102,'residential',NULL,1,false,110,30,{p('LINESTRING(18.07 59.32,18.08 59.32)')}),
        ({AR},2,1,100,'primary','Main',2,true,80,50,{p('LINESTRING(18.07 59.32,18.06 59.32)')})""")
    con.execute("CREATE TABLE driving.edge_graph(from_edge BIGINT, to_edge BIGINT, via_edge BIGINT, cost DOUBLE)")
    con.execute(f"INSERT INTO driving.edge_graph VALUES ({A},{B},{B},1.0),({A},{C},{C},1.0)")
    con.close()
    out = tmp_path / "gmns.duckdb"
    to_gmns(str(src), str(out))
    return out


def test_build_lane_graph(tmp_path):
    db = _gmns(tmp_path)
    r = build_lane_graph(str(db))
    assert r["lanes"] == 6 and r["lane_change"] >= 2 and r["turn"] >= 2   # A/AR are 2-lane → lane-change
    con = duckdb.connect(str(db), read_only=True)
    kinds = {row[0] for row in con.execute("SELECT DISTINCT kind FROM lane_driving.lane_edges").fetchall()}
    assert "lane_change" in kinds
    # a lane-change edge exists between A's two lanes
    n = con.execute("SELECT count(*) FROM lane_driving.lane_edges WHERE kind='lane_change' "
                    f"AND from_lane='{A}_1' AND to_lane='{A}_2'").fetchone()[0]
    assert n == 1


def test_route_lanes(tmp_path):
    db = _gmns(tmp_path)
    build_lane_graph(str(db))
    r = route_lanes(str(db), f"{A}_1", f"{C}_1")
    assert r["lanes"][0] == f"{A}_1" and r["lanes"][-1] == f"{C}_1"       # reaches the target lane
    assert r["cost"] > 0 and r["geometry"].startswith("LINESTRING")
    assert isinstance(r["maneuvers"], list)


def test_route_by_edge_id(tmp_path):
    # passing an edge_id resolves to its lane 1
    db = _gmns(tmp_path)
    r = route_lanes(str(db), A, B)                                        # A, B as edge_ids
    assert r["lanes"] and r["lanes"][0] == f"{A}_1" and r["lanes"][-1] == f"{B}_1"


def test_route_without_prebuild(tmp_path):
    # route_lanes builds the graph in-memory if lane_edges isn't persisted
    db = _gmns(tmp_path)
    r = route_lanes(str(db), f"{A}_1", f"{B}_1")
    assert r["lanes"] and r["lanes"][-1] == f"{B}_1"


def test_unreachable(tmp_path):
    db = _gmns(tmp_path)
    r = route_lanes(str(db), f"{A}_1", "999999999999999999_9")
    assert r == {"lanes": [], "cost": None, "geometry": None, "maneuvers": []}


def test_bad_db_raises(tmp_path):
    bare = tmp_path / "bare.duckdb"
    duckdb.connect(str(bare)).close()
    with pytest.raises(ValueError, match="lane"):
        build_lane_graph(str(bare))
