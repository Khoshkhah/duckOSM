"""The Python API of duckOSM on the Rust code (the module ``duckosm._rust``). docs/design/all_in_rust.md, step 6.

The functions keep the names, the arguments and the results of the Python code. Each one calls Rust, except for the
cases that the Rust code does not have; these call the Python code (the module named in each function).

TODO (step 6, before the Python engine code goes):
- The build: ``DuckOSM.run()`` runs the Python steps with the Rust network step, as before. The whole build in Rust
  (``duckosm --config``) is not yet equal to it: no ``features.ocean`` (``options.sea``), no
  ``visualization.level_heads`` and the other tables of the level edits, no ``main.building_faces`` in some builds,
  other tables with a fixes file of the page (tests/test_page_fixes.py), and ``main.visualization_metadata`` gives
  ``duckosm 0.7.0`` as the version. tests/test_engine_rust.py, test_levels_in_build.py, test_ask_schema.py,
  test_editor_changes.py and test_page_fixes.py fail with it.
- GMNS (group C): the command ``duckosm gmns`` is Rust; the functions ``to_gmns``, ``to_meso``, ``to_micro``,
  ``check_gmns`` and ``to_sumo`` with ``bus_edges``, ``edge_attrs`` or ``lanes`` still call the Python code.
- The commands with no Rust port yet: ``levels``, ``bus``, ``multimodal``, ``clip-pbf`` (see ``duckosm/_main.py``).
- ``to_networkx``, ``to_networkx_nodes`` and ``write_graph`` give networkx graphs: they stay Python.
- ``to_sumo`` with a ``dict`` config or ``netconvert_bin``, ``to_railml`` with other ``rail_types``,
  ``route`` with a ``graph`` and ``route_multimodal`` with ``_points``.
"""

import json
import os
from pathlib import Path


def _rust():
    from duckosm import _rust
    return _rust


def _db(con):
    """The file of a database: a path, or a DuckDB connection to a database file. None: an in-memory database (the Rust
    code cannot read it, so the Python code runs)."""
    if isinstance(con, (str, os.PathLike)):
        return str(con)
    row = con.execute("SELECT path FROM duckdb_databases() WHERE database_name = current_database()").fetchone()
    return row[0] if row and row[0] and row[0] != ":memory:" else None


def _paths(d, keys):
    for k in keys:
        if d.get(k) is not None:
            d[k] = Path(d[k])
    return d


# ---- exports -----------------------------------------------------------------------------------------------------

def to_sumo(con, out_dir, mode: str = "driving", net_name: str = "network", run_netconvert: bool = True,
            netconvert_bin: str = None, connections: bool = True, config=None, bus_edges: bool = False,
            edge_attrs: dict = None, lanes: dict = None):
    """See :func:`duckosm.sumo.to_sumo`."""
    if bus_edges or edge_attrs or lanes or isinstance(config, dict) or netconvert_bin or _db(con) is None:
        from duckosm import sumo
        return sumo.to_sumo(con, out_dir, mode=mode, net_name=net_name, run_netconvert=run_netconvert,
                            netconvert_bin=netconvert_bin, connections=connections, config=config,
                            bus_edges=bus_edges, edge_attrs=edge_attrs, lanes=lanes)
    r = json.loads(_rust().to_sumo(_db(con), str(out_dir), mode, net_name, connections,
                                   None if config is None else str(config), run_netconvert))
    return _paths(r, ("nod", "edg", "con", "netccfg", "net"))


def to_gis(con, out, modes=None, fmt: str = "gpkg", boundary: bool = True, name: str = None,
           srs: str = "EPSG:4326", ogr2ogr_bin: str = None):
    """See :func:`duckosm.gis.to_gis`. ``ogr2ogr_bin`` is not used: Rust writes the GeoPackage through DuckDB."""
    if _db(con) is None:
        from duckosm import gis
        return gis.to_gis(con, out, modes, fmt, boundary, name, srs, ogr2ogr_bin)
    modes = [modes] if isinstance(modes, str) else list(modes or [])
    r = json.loads(_rust().to_gis(_db(con), str(out), modes, fmt, boundary, name, srs))
    r["files"] = [Path(f) for f in r["files"]]
    return _paths(r, ("path", "dir"))


def to_matsim(source, out_path, mode="driving", crs=None, gzip=True):
    """See :func:`duckosm.matsim.to_matsim`."""
    if not isinstance(mode, str):
        mode = ",".join(mode)
    return json.loads(_rust().to_matsim(_db(source), str(out_path), mode, crs, gzip))


def to_matsim_lanes(gmns_db, out_dir=".", mode="driving", signals=True, cycle_s=90):
    """See :func:`duckosm.matsim_lanes.to_matsim_lanes`."""
    return json.loads(_rust().to_matsim_lanes(_db(gmns_db), str(out_dir), mode, signals, int(cycle_s)))


def to_opendrive(source, out_path, mode="driving", crs=None, junctions=False):
    """See :func:`duckosm.opendrive.to_opendrive`."""
    return json.loads(_rust().to_opendrive(_db(source), str(out_path), mode, crs, junctions))


_RAIL = ("rail", "light_rail", "subway", "tram", "narrow_gauge", "funicular")


def to_railml(source, out_path, rail_types=_RAIL):
    """See :func:`duckosm.railml.to_railml`."""
    if tuple(rail_types) != _RAIL:
        from duckosm import railml
        return railml.to_railml(source, out_path, rail_types)
    return json.loads(_rust().to_railml(_db(source), str(out_path)))


def to_lanelet2(gmns_db, out_path, mode="driving", snap_m=0.01):
    """See :func:`duckosm.lanelet2.to_lanelet2`."""
    return json.loads(_rust().to_lanelet2(_db(gmns_db), str(out_path), mode, snap_m))


def to_elevation(db, dem=None, source="auto", modes=None, nodata_fill=0.0, suffix="",
                 vertical_datum=None, license=None, reference=True, product=None):
    """See :func:`duckosm.elevation.to_elevation`."""
    modes = [modes] if isinstance(modes, str) else list(modes or [])
    return json.loads(_rust().to_elevation(_db(db), None if dem is None else str(dem), source, modes, float(nodata_fill),
                                           suffix, product, vertical_datum, license, reference))


# ---- routing -----------------------------------------------------------------------------------------------------

class Router:
    """Build the routing graph of one mode once, then answer many shortest-route queries. See
    :class:`duckosm.routing.Router`; the graph is in Rust, so there is no ``.graph``."""

    def __init__(self, con, mode: str = "driving", weight: str = "time"):
        self.con, self.mode, self.weight = con, mode, weight
        db = _db(con)
        if db is None:
            from duckosm import routing
            self._py = routing.Router(con, mode, weight)
        else:
            self._py, self._r = None, _rust().Router(db, mode, weight)

    def route(self, from_edge, to_edge):
        """Shortest route between two edge ids: the dict of :func:`route`, or None."""
        if self._py is not None:
            return self._py.route(from_edge, to_edge)
        try:
            return json.loads(self._r.route(int(from_edge), int(to_edge)))
        except RuntimeError as e:
            if "not in the" in str(e):        # as networkx: a ValueError
                raise ValueError(str(e)) from None
            raise


def route(con, from_edge, to_edge, mode: str = "driving", weight: str = "time", graph=None):
    """See :func:`duckosm.routing.route`. With a networkx ``graph``, the Python code."""
    if graph is not None or _db(con) is None:
        from duckosm import routing
        return routing.route(con, from_edge, to_edge, mode=mode, weight=weight, graph=graph)
    return Router(con, mode, weight).route(from_edge, to_edge)


def route_multimodal(con, src_node, dst_node, schema: str = "mm", start_mode: str = "walking",
                     end_mode: str = "walking", enforce_sequence: bool = True, allowed_modes=None, _points=None):
    """See :func:`duckosm.routing.route_multimodal`."""
    if _points is not None or _db(con) is None:
        from duckosm import routing
        return routing.route_multimodal(con, src_node, dst_node, schema, start_mode, end_mode, enforce_sequence,
                                        allowed_modes, _points)
    modes = None if allowed_modes is None else list(allowed_modes)
    return json.loads(_rust().route_multimodal(_db(con), int(src_node), int(dst_node), schema, start_mode, end_mode,
                                               enforce_sequence, modes))


def route_points(con, a, b, mode="driving", weight="time", radius_m=50.0, access_kmh=4.5):
    """See :func:`duckosm.point_routing.route_points`."""
    if _db(con) is None:
        from duckosm import point_routing
        return point_routing.route_points(con, a, b, mode, weight, radius_m, access_kmh)
    return json.loads(_rust().route_points(_db(con), tuple(a), tuple(b), mode, weight, float(radius_m), float(access_kmh)))


def route_multimodal_points(con, a, b, radius_m=50.0, access_kmh=4.5, schema="mm", allowed_modes=None,
                            enforce_sequence=True):
    """See :func:`duckosm.point_routing.route_multimodal_points`."""
    if _db(con) is None:
        from duckosm import point_routing
        return point_routing.route_multimodal_points(con, a, b, radius_m, access_kmh, schema, allowed_modes, enforce_sequence)
    modes = None if allowed_modes is None else list(allowed_modes)
    return json.loads(_rust().route_multimodal_points(_db(con), tuple(a), tuple(b), float(radius_m), float(access_kmh),
                                                      schema, modes, enforce_sequence))


def directions(con, route, mode="driving"):
    """See :func:`duckosm.directions.directions`."""
    if _db(con) is None:
        from duckosm import directions as d
        return d.directions(con, route, mode)
    return json.loads(_rust().directions(_db(con), json.dumps(route), mode))


def build_lane_graph(gmns_db, mode="driving"):
    """See :func:`duckosm.lane_routing.build_lane_graph`."""
    lanes, lc, turn = _rust().build_lane_graph(_db(gmns_db), mode)
    return {"lanes": lanes, "lane_change": lc, "turn": turn, "edges": lc + turn}


def route_lanes(gmns_db, from_lane, to_lane, mode="driving"):
    """See :func:`duckosm.lane_routing.route_lanes`."""
    return json.loads(_rust().route_lanes(_db(gmns_db), str(from_lane), str(to_lane), mode))
