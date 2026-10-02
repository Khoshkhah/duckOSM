"""Where a footway meets a road, the connector is data (docs/design/gmns_walk_joins.md)."""
import pytest

pytest.importorskip("pandas")
from tests.test_gmns_walking_frame import M, _run  # noqa: E402


def _joins(c):
    """(from lane, to lane, width, length in metres) of the walking connectors."""
    return c.execute("""SELECT from_lane_id, to_lane_id, width,
                               sqrt(power((ST_X(ST_EndPoint(geom)) - ST_X(ST_StartPoint(geom))) * cos(radians(59.32)) * 111320, 2)
                                    + power((ST_Y(ST_EndPoint(geom)) - ST_Y(ST_StartPoint(geom))) * 111320, 2))
                        FROM gmns_walking.lane_connector ORDER BY connector_id""").fetchall()


def test_a_footway_on_the_road_line_is_joined_to_a_road_lane_at_each_end(tmp_path):
    # the footway runs on the road's line (walk_frame off), its ends on nodes 1 and 2; the road lanes sit beside that line
    j = _joins(_run(tmp_path, 59.32, road_in_walking=False, walk_frame=False))
    assert len(j) == 2 and {r[1] for r in j} == {"9001_1"}          # one per footway end
    assert all(0.3 < r[3] <= 6 and r[2] == 2.0 for r in j)         # a short straight segment, as wide as the footway
    assert all(r[0].split("_")[0] != "9001" for r in j)             # leaving a road lane


def test_a_footway_far_from_the_road_ends_gets_no_join(tmp_path):
    # 20 m from the road's lane ends: no link OSM does not map is invented
    assert _joins(_run(tmp_path, 59.32 + 20 / M, road_in_walking=False, walk_frame=False)) == []
