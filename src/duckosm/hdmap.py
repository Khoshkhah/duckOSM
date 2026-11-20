"""``duckosm.hdmap``: automated driving and HD maps (docs/proposals/python-sdk.md, section 5.7).

Today: ``to_opendrive``, ``to_lanelet2``, ``lane_graph`` (``build_lane_graph``), ``route_lanes``. Later:
``to_commonroad``. Lanelet2 and the lane graph read a GMNS database (:func:`duckosm.planning.to_gmns`).
"""
from duckosm._api import build_lane_graph, route_lanes, to_lanelet2  # noqa: F401
from duckosm.network import resolve

lane_graph = build_lane_graph


def to_opendrive(net, out_path, mode=None, crs=None, junctions=False):
    """The ASAM OpenDRIVE file (``.xodr``) of one mode (experimental). ``junctions``: also the junctions, from the
    movements. Returns the numbers.

    >>> duckosm.hdmap.to_opendrive(net.driving, "out/monaco.xodr")"""
    from duckosm import _api
    db, mode = resolve(net, mode)
    return _api.to_opendrive(db, out_path, mode=mode, crs=crs, junctions=junctions)
