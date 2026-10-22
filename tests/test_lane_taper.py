"""A two-way road's lanes move sideways at a joint over a taper, not in one step (gmns._taper_wkt)."""
from shapely import wkt

from duckosm.gmns import _taper_wkt

M = 111320.0


def test_taper_moves_the_ends_and_leaves_the_middle():
    g = wkt.loads(_taper_wkt("LINESTRING (18.0 59.0, 18.001 59.0)", 1.0, 10.0, -0.5, 6.0))    # about 57 m, east
    dy = lambda y: (y - 59.0) * M                                                              # noqa: E731
    assert abs(dy(g.coords[0][1]) - 1.0) < 0.01          # left of an east-going line is north
    assert abs(dy(g.coords[-1][1]) + 0.5) < 0.01
    assert abs(dy(g.interpolate(0.5, normalized=True).y)) < 0.01
