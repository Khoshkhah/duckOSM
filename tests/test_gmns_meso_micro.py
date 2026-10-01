"""Meso and micro networks: lengths are metres, and every link starts and ends at its nodes.

A network at 59.3 N (a degree of longitude is half a degree of latitude there), where lengths measured in
degrees * 111320 were up to twice too long, and trimmed meso sections left their connectors short of them.
"""
import math

import duckdb
import pytest

pytest.importorskip("pandas")
from duckosm.gmns import to_gmns, to_meso, to_micro  # noqa: E402
from tests.test_gmns import _source  # noqa: E402

SPHEROID = "ST_Length_Spheroid(ST_FlipCoordinates(geom))"


@pytest.fixture(scope="module")
def con(tmp_path_factory):
    d = tmp_path_factory.mktemp("mm")
    _source(d / "src.duckdb")
    to_gmns(str(d / "src.duckdb"), str(d / "out.duckdb"))
    to_meso(str(d / "out.duckdb"))
    to_micro(str(d / "out.duckdb"))
    c = duckdb.connect(str(d / "out.duckdb"), read_only=True)
    c.execute("LOAD spatial")
    return c


@pytest.mark.parametrize("table", ["meso_driving.meso_link", "micro_driving.micro_link"])
def test_lengths_are_metres_of_the_links_own_geometry(con, table):
    n, bad, nulls = con.execute(f"""SELECT count(*),
        count(*) FILTER (WHERE abs(length / NULLIF({SPHEROID}, 0) - 1) > 0.01),
        count(*) FILTER (WHERE length IS NULL OR isnan(length)) FROM {table}""").fetchone()
    assert n > 0 and bad == 0 and nulls == 0


@pytest.mark.parametrize("table, node_table, node_key", [
    ("meso_driving.meso_link", "meso_driving.meso_node", "node_id"),
    ("micro_driving.micro_link", "micro_driving.micro_node", "node_id")])
def test_every_link_starts_and_ends_at_its_nodes(con, table, node_table, node_key):
    bad = con.execute(f"""SELECT count(*) FROM {table} l
        JOIN {node_table} a ON a.{node_key} = l.from_node_id JOIN {node_table} b ON b.{node_key} = l.to_node_id
        WHERE abs(ST_X(ST_StartPoint(l.geom)) - a.x_coord) > 1e-7 OR abs(ST_Y(ST_StartPoint(l.geom)) - a.y_coord) > 1e-7
           OR abs(ST_X(ST_EndPoint(l.geom)) - b.x_coord) > 1e-7 OR abs(ST_Y(ST_EndPoint(l.geom)) - b.y_coord) > 1e-7""").fetchone()[0]
    assert bad == 0
    assert con.execute(f"SELECT count(*) FROM {table}").fetchone()[0] > 0


def test_meso_has_a_connector_per_movement_and_a_section_per_link(con):
    assert con.execute("SELECT count(*) FROM meso_driving.meso_link WHERE meso_type = 'movement'").fetchone() == \
        con.execute("SELECT count(*) FROM gmns_driving.movement").fetchone()
    assert con.execute("SELECT count(*) FROM meso_driving.meso_link WHERE meso_type = 'normal'").fetchone() == \
        con.execute("SELECT count(*) FROM gmns_driving.link").fetchone()


def test_a_lane_has_as_many_cells_as_its_length_in_7_m_pieces(con):
    for lane_id, length, cells in con.execute(f"""SELECT l.lane_id, ST_Length_Spheroid(ST_FlipCoordinates(l.geom)),
        (SELECT count(*) FROM micro_driving.micro_link c WHERE c.cell_type = 'normal' AND c.link_id LIKE 'C' || l.lane_id || '#%')
        FROM gmns_driving.lane l""").fetchall():
        assert cells == max(1, math.ceil(length / 7.0)), lane_id
