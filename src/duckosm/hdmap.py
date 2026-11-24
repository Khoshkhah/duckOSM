"""``duckosm.hdmap``: automated driving and HD maps (docs/proposals/python-sdk.md, section 5.7).

``to_opendrive``, ``to_lanelet2``, ``lane_graph`` (``build_lane_graph``), ``route_lanes``, ``to_commonroad`` (a
CommonRoad map, with commonroad-io) and ``surfaces`` (the road surface of each lane and each junction). Lanelet2, CommonRoad and the lane graph read a GMNS database
(:func:`duckosm.planning.to_gmns`). pip extra: ``duckosm[hdmap]``.
"""
from duckosm._api import build_lane_graph, route_lanes, to_lanelet2  # noqa: F401
from duckosm.network import resolve

lane_graph = build_lane_graph


def to_opendrive(net, out_path, mode=None, crs=None, junctions=False):
    """The ASAM OpenDRIVE file (``.xodr``) of one mode (experimental). ``junctions``: also the junctions, from the
    movements of a GMNS database. ``net`` can be a network, a mode view or a GMNS database
    (:func:`duckosm.planning.to_gmns`). With ``junctions=True`` and a network, this function first writes the GMNS
    database of the mode to a temporary folder and reads the roads and the junctions from it. To keep that file, make
    it with :func:`duckosm.planning.to_gmns` and pass its path. Returns the numbers.

    >>> duckosm.hdmap.to_opendrive(net.driving, "out/monaco.xodr")
    >>> duckosm.hdmap.to_opendrive(net.driving, "out/monaco_junctions.xodr", junctions=True)"""
    from duckosm import _api
    db, mode = resolve(net, mode)
    path = _api._db(db)
    if not junctions or path is None or _has_table(path, f"gmns_{mode}", "movement"):
        return _api.to_opendrive(db, out_path, mode=mode, crs=crs, junctions=junctions)
    import os
    import tempfile

    from duckosm.planning import to_gmns
    with tempfile.TemporaryDirectory() as tmp:
        gmns = os.path.join(tmp, "gmns.duckdb")
        to_gmns(path, gmns, mode=mode)
        return _api.to_opendrive(gmns, out_path, mode=mode, crs=crs, junctions=True)


def _has_table(path, schema, table):
    import duckdb
    con = duckdb.connect(str(path), read_only=True)
    try:
        return con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name = ? AND table_name = ?",
                           [schema, table]).fetchone()[0] > 0
    finally:
        con.close()


def surfaces(gmns_db, mode="driving"):
    """The road surfaces of a GMNS database (:func:`duckosm.planning.to_gmns`) as two GeoDataFrames (EPSG:4326):
    ``(lanes, junctions)``. ``lanes``: one polygon for each lane, its center line to the edge of the junction
    (``geom_cut``) with the width of the lane (3.25 m if the lane has no width), with flat ends. Columns ``lane_id``,
    ``edge_id``, ``lane_num``, ``width``, ``allowed_uses``, ``geometry``. ``junctions``: one polygon for each junction
    node, the union of its lane connectors with their widths. Columns ``node_id``, ``connectors``, ``geometry``. DuckDB
    computes the polygons in the UTM zone of the area.

    >>> lanes, junctions = duckosm.hdmap.surfaces("out/gmns.duckdb")"""
    import geopandas as gpd
    import pandas as pd

    from duckosm import _rust
    lanes, junctions = _rust.hdmap_surfaces(str(gmns_db), mode)
    lanes = pd.DataFrame(lanes, columns=["lane_id", "edge_id", "lane_num", "width", "allowed_uses", "wkb"])
    junctions = pd.DataFrame(junctions, columns=["node_id", "connectors", "wkb"])

    def gdf(df):
        return gpd.GeoDataFrame(df.drop(columns="wkb"), geometry=gpd.GeoSeries.from_wkb(df.wkb.map(bytes)), crs="EPSG:4326")
    return gdf(lanes), gdf(junctions)

def to_commonroad(gmns_db, path, mode="driving", country_id="ZAM", map_name=None, author="duckOSM", affiliation=""):
    """A CommonRoad scenario map (``.xml``, commonroad-io) of the lanes of a GMNS database (:func:`duckosm.planning.to_gmns`):
    one lanelet for each lane (its center line to the edge of the junction, ``geom_cut``, and its width) and one for
    each lane connector through a junction, with the predecessors, the successors and the lanes on each side. The
    coordinates are metres east and north of the center of the area (``location`` has its latitude and longitude).
    Returns ``{path, lanelets, lanes, connectors, dropped, ids}``: ``lanes`` and ``connectors`` are the rows of the GMNS
    database; ``lanelets`` is the number of lanelets in the file. ``dropped`` is the number of lanes and connectors
    that are not in the file, because their left or right border is not one line (a lane that is too short or bends too
    much for its width: 231 of 7450 on Monaco). ``ids`` is a DataFrame of ``lanelet_id`` → GMNS ``lane_id`` or
    ``connector_id`` and the ``edge_id``, only for the lanelets in the file.

    >>> duckosm.hdmap.to_commonroad("out/gmns.duckdb", "out/monaco_commonroad.xml")"""
    from pathlib import Path

    import numpy as np
    import pandas as pd

    from duckosm import _rust
    from duckosm.errors import require
    require("commonroad", "duckosm.hdmap.to_commonroad")
    from commonroad.common.file_writer import CommonRoadFileWriter, OverwriteExistingFile
    from commonroad.planning.planning_problem import PlanningProblemSet
    from commonroad.scenario.lanelet import Lanelet, LaneletNetwork
    from commonroad.scenario.scenario import Scenario, ScenarioID, Tag
    try:                                       # commonroad-io 2024 and later
        from commonroad.common.common_lanelet import LaneletType, RoadUser
        from commonroad.common.common_scenario import Location
        from commonroad.common.util import FileFormat
        xml = {"file_format": FileFormat.XML}  # the default is protobuf
    except ImportError:                        # commonroad-io 2022 and 2023 (pip takes it when protobuf 3.20 cannot be installed)
        from commonroad.scenario.lanelet import LaneletType, RoadUser
        from commonroad.scenario.scenario import Location
        xml = {}

    # the lanelets, their borders and their links come from Rust; here only the commonroad-io objects are made
    (lon0, lat0), n_lanes, n_conns, rows = _rust.commonroad_lanelets(str(gmns_db), mode)
    kinds = {"bus": (LaneletType.BUS_LANE, RoadUser.BUS), "bike": (LaneletType.BICYCLE_LANE, RoadUser.BICYCLE),
             "urban": (LaneletType.URBAN, RoadUser.VEHICLE)}
    lanelets = []
    for i, left, center, right, pred, succ, adj_l, adj_r, kind, _, _ in rows:
        k, user = kinds[kind]
        lanelets.append(Lanelet(np.array(left), np.array(center), np.array(right), i, predecessor=pred, successor=succ,
                                adjacent_left=adj_l, adjacent_left_same_direction=True if adj_l else None,
                                adjacent_right=adj_r, adjacent_right_same_direction=True if adj_r else None,
                                lanelet_type={k}, user_one_way={user}))
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    import re
    name = re.sub(r"[^A-Za-z0-9]", "", map_name or path.name.split(".")[0])[:32] or "duckOSM"   # "_" and "-" split the id
    scenario = Scenario(0.1, ScenarioID(country_id=country_id, map_name=name, map_id=1))
    scenario.add_objects(LaneletNetwork.create_from_lanelet_list(lanelets))
    location = Location(gps_latitude=lat0, gps_longitude=lon0)
    CommonRoadFileWriter(scenario, PlanningProblemSet(), author=author, affiliation=affiliation, source="duckOSM",
                         tags={Tag.URBAN}, location=location, **xml).write_to_file(str(path), OverwriteExistingFile.ALWAYS)
    ids = pd.DataFrame([(r[0], r[9], r[10]) for r in rows], columns=["lanelet_id", "gmns_id", "edge_id"])
    return {"path": path, "lanelets": len(lanelets), "lanes": n_lanes, "connectors": n_conns,
            "dropped": n_lanes + n_conns - len(lanelets), "ids": ids}
