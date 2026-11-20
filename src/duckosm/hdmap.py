"""``duckosm.hdmap``: automated driving and HD maps (docs/proposals/python-sdk.md, section 5.7).

``to_opendrive``, ``to_lanelet2``, ``lane_graph`` (``build_lane_graph``), ``route_lanes`` and ``to_commonroad`` (a
CommonRoad map, with commonroad-io). Lanelet2, CommonRoad and the lane graph read a GMNS database
(:func:`duckosm.planning.to_gmns`). pip extra: ``duckosm[hdmap]``.
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


def _local(geoms, lon0, lat0):
    """The lon/lat shapely geometries in metres, east and north of (lon0, lat0) (a local plane; numpy, no loop)."""
    import math

    import numpy as np
    import shapely
    kx, ky = 111_320.0 * math.cos(math.radians(lat0)), 110_574.0
    return shapely.transform(geoms, lambda c: np.column_stack([(c[:, 0] - lon0) * kx, (c[:, 1] - lat0) * ky]))


def to_commonroad(gmns_db, path, mode="driving", country_id="ZAM", map_name=None, author="duckOSM", affiliation=""):
    """A CommonRoad scenario map (``.xml``, commonroad-io) of the lanes of a GMNS database (:func:`duckosm.planning.to_gmns`):
    one lanelet for each lane (its center line to the edge of the junction, ``geom_cut``, and its width) and one for
    each lane connector through a junction, with the predecessors, the successors and the lanes on each side. The
    coordinates are metres east and north of the center of the area (``location`` has its latitude and longitude).
    Returns ``{path, lanelets, lanes, connectors, ids}``: ``ids`` is a DataFrame of ``lanelet_id`` → GMNS ``lane_id``
    or ``connector_id`` and the ``edge_id``.

    >>> duckosm.hdmap.to_commonroad("out/gmns.duckdb", "out/monaco_commonroad.xml")"""
    from pathlib import Path

    import duckdb
    import numpy as np
    import pandas as pd
    import shapely

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

    g = f"gmns_{mode}"
    con = duckdb.connect(str(gmns_db), read_only=True)
    try:
        con.load_extension("spatial")
        lanes = con.execute(f"""SELECT la.lane_id AS key, la.link_id AS edge_id, la.lane_num, coalesce(la.width, 3.25) AS width,
                                       la.allowed_uses AS uses, ST_AsWKB(coalesce(la.geom_cut, la.geom)) AS wkb
                                FROM {g}.lane la WHERE coalesce(la.geom_cut, la.geom) IS NOT NULL ORDER BY la.lane_id""").df()
        conns = con.execute(f"""SELECT c.connector_id AS key, c.from_lane_id, c.to_lane_id, coalesce(c.width, 3.25) AS width,
                                       NULL AS uses, ST_AsWKB(c.geom) AS wkb
                                FROM {g}.lane_connector c WHERE c.geom IS NOT NULL ORDER BY c.connector_id""").df() \
            if con.execute(f"SELECT count(*) FROM duckdb_tables() WHERE schema_name = '{g}' AND table_name = 'lane_connector'").fetchone()[0] \
            else pd.DataFrame(columns=["key", "from_lane_id", "to_lane_id", "width", "uses", "wkb"])
    finally:
        con.close()
    rows = pd.concat([lanes, conns], ignore_index=True)
    rows["lanelet_id"] = np.arange(1, len(rows) + 1)
    lines = shapely.from_wkb(rows.wkb.map(bytes).to_numpy())
    lon0, lat0 = shapely.centroid(shapely.union_all(shapely.envelope(lines))).coords[0]
    lines = _local(lines, lon0, lat0)
    half = rows.width.to_numpy(float) / 2
    left, right = shapely.offset_curve(lines, half), shapely.offset_curve(lines, -half)

    lane_id = dict(zip(lanes.key, map(int, rows.lanelet_id[:len(lanes)])))
    succ, pred, adj = {}, {}, {}
    for c, i in zip(conns.itertuples(), rows.lanelet_id[len(lanes):]):
        a, b = lane_id.get(c.from_lane_id), lane_id.get(c.to_lane_id)
        if a and b:
            succ.setdefault(a, []).append(int(i)), pred.setdefault(int(i), []).append(a)
            succ.setdefault(int(i), []).append(b), pred.setdefault(b, []).append(int(i))
    num = {(int(e), int(n)): int(i) for e, n, i in zip(lanes.edge_id, lanes.lane_num, rows.lanelet_id)}
    for (e, n), i in num.items():              # lane 1 is the leftmost: lane n + 1 is on its right, same direction
        j = num.get((e, n + 1))
        if j:
            adj.setdefault(i, {})["right"], adj.setdefault(j, {})["left"] = j, i

    def resample(line, n):
        return np.array([p.coords[0] for p in shapely.line_interpolate_point(line, np.linspace(0, 1, n), normalized=True)])

    kinds = {"bus": (LaneletType.BUS_LANE, RoadUser.BUS), "bike": (LaneletType.BICYCLE_LANE, RoadUser.BICYCLE)}
    sides = {}                                 # lanelet_id: (left, right) with the same number of points
    for i, l, rt in zip(rows.lanelet_id, left, right):
        if l is not None and rt is not None and l.geom_type == rt.geom_type == "LineString" and not (l.is_empty or rt.is_empty):
            n = max(len(l.coords), len(rt.coords), 2)
            sides[int(i)] = (resample(l, n), resample(rt, n))
    lanelets = []
    for r in rows.itertuples():
        i = int(r.lanelet_id)
        if i not in sides:
            continue
        lv, rv = sides[i]
        kind, user = kinds.get(r.uses.split(",")[0] if isinstance(r.uses, str) else "", (LaneletType.URBAN, RoadUser.VEHICLE))
        a = {k: v for k, v in adj.get(i, {}).items() if v in sides}
        lanelets.append(Lanelet(lv, (lv + rv) / 2, rv, i, predecessor=[x for x in pred.get(i, []) if x in sides],
                                successor=[x for x in succ.get(i, []) if x in sides],
                                adjacent_left=a.get("left"), adjacent_left_same_direction=True if "left" in a else None,
                                adjacent_right=a.get("right"), adjacent_right_same_direction=True if "right" in a else None,
                                lanelet_type={kind}, user_one_way={user}))
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    import re
    name = re.sub(r"[^A-Za-z0-9]", "", map_name or path.name.split(".")[0])[:32] or "duckOSM"   # "_" and "-" split the id
    scenario = Scenario(0.1, ScenarioID(country_id=country_id, map_name=name, map_id=1))
    scenario.add_objects(LaneletNetwork.create_from_lanelet_list(lanelets))
    location = Location(gps_latitude=lat0, gps_longitude=lon0)
    CommonRoadFileWriter(scenario, PlanningProblemSet(), author=author, affiliation=affiliation, source="duckOSM",
                         tags={Tag.URBAN}, location=location, **xml).write_to_file(str(path), OverwriteExistingFile.ALWAYS)
    ids = pd.DataFrame({"lanelet_id": rows.lanelet_id, "gmns_id": rows.key,
                        "edge_id": pd.concat([lanes.edge_id, conns.from_lane_id.str.split("_").str[0].astype("int64")],
                                             ignore_index=True)})
    return {"path": path, "lanelets": len(lanelets), "lanes": len(lanes), "connectors": len(conns), "ids": ids}
