"""The Python API of duckOSM on the Rust code (the module ``duckosm._rust``). docs/design/all_in_rust.md, step 6.

The functions keep the names, the arguments and the results of the Python code. Each one calls Rust, except for the
cases that the Rust code does not have; these call the Python code (the module named in each function).

TODO (step 6, before the Python engine code goes):
- GMNS (group C): ``to_gmns``, ``to_meso``, ``to_micro``, ``check_gmns``, and ``to_sumo`` with ``bus_edges``,
  ``edge_attrs`` or ``lanes`` (the GMNS parts of the SUMO export).
- The maps (group F): ``duckosm viz``, ``route-map``, ``levels-edit``, ``gmns-viz``, ``gmns-map``.
- The commands with no Rust port yet: ``levels``, ``bus``, ``multimodal``, ``clip-pbf`` (see ``duckosm/_main.py``).
- ``to_networkx``, ``to_networkx_nodes`` and ``write_graph`` give networkx graphs: they stay Python.
- ``to_sumo`` with a ``dict`` config or ``netconvert_bin``, ``to_railml`` with other ``rail_types``,
  ``route`` with a ``graph`` and ``route_multimodal`` with ``_points``.
"""

import dataclasses
import json
import os
import tempfile
from pathlib import Path


def _rust():
    from duckosm import _rust
    return _rust


def _db(con):
    """The file of a database: a path, or a DuckDB connection to a database file."""
    if isinstance(con, (str, os.PathLike)):
        return str(con)
    row = con.execute("SELECT path FROM duckdb_databases() WHERE database_name = current_database()").fetchone()
    if not row or not row[0]:
        raise ValueError("the Rust code reads a database file: give a connection to a file, not to an in-memory database")
    return row[0]


def _paths(d, keys):
    for k in keys:
        if d.get(k) is not None:
            d[k] = Path(d[k])
    return d


# ---- the build -------------------------------------------------------------------------------------------------

def _config_yaml(config):
    """The config as the YAML text of a config file. Every value is written, so the defaults are those of Python."""
    import yaml
    data = {}
    for f in dataclasses.fields(config):
        v = getattr(config, f.name)
        if dataclasses.is_dataclass(v) and not isinstance(v, type):
            section = {k: x for k, x in dataclasses.asdict(v).items() if x is not None}
            section.pop("engine", None)        # Python only: options.engine
            if section:
                data[f.name] = section
        elif v not in (None, "", []):
            data[f.name] = v
    # the sections say it already
    for top, (sec, key) in {"pbf_path": ("source", "pbf_path"), "boundary_path": ("boundary", "path"),
                            "h3_cell": ("boundary", "h3_cell")}.items():
        if top in data and data.get(sec, {}).get(key) is not None:
            del data[top]
    return yaml.safe_dump(data, sort_keys=False)


def build(config):
    """``DuckOSM(config).run()`` in Rust: ``duckosm build --config``. Returns the path of the database."""
    with tempfile.TemporaryDirectory() as d:
        path = Path(d) / "config.yaml"
        path.write_text(_config_yaml(config))
        code = _rust().cli(["duckosm", "build", "--config", str(path)])
    if code:
        raise RuntimeError(f"duckosm build failed (exit code {code})")
    return config.get_db_path()


# ---- exports -----------------------------------------------------------------------------------------------------

def to_sumo(con, out_dir, mode: str = "driving", net_name: str = "network", run_netconvert: bool = True,
            netconvert_bin: str = None, connections: bool = True, config=None, bus_edges: bool = False,
            edge_attrs: dict = None, lanes: dict = None):
    """See :func:`duckosm.sumo.to_sumo`."""
    if bus_edges or edge_attrs or lanes or isinstance(config, dict) or netconvert_bin:
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
        self._r = _rust().Router(_db(con), mode, weight)

    def route(self, from_edge, to_edge):
        """Shortest route between two edge ids: the dict of :func:`route`, or None."""
        try:
            return json.loads(self._r.route(int(from_edge), int(to_edge)))
        except RuntimeError as e:
            if "not in the" in str(e):        # as networkx: a ValueError
                raise ValueError(str(e)) from None
            raise


def route(con, from_edge, to_edge, mode: str = "driving", weight: str = "time", graph=None):
    """See :func:`duckosm.routing.route`. With a networkx ``graph``, the Python code."""
    if graph is not None:
        from duckosm import routing
        return routing.route(con, from_edge, to_edge, mode=mode, weight=weight, graph=graph)
    return Router(con, mode, weight).route(from_edge, to_edge)


def route_multimodal(con, src_node, dst_node, schema: str = "mm", start_mode: str = "walking",
                     end_mode: str = "walking", enforce_sequence: bool = True, allowed_modes=None, _points=None):
    """See :func:`duckosm.routing.route_multimodal`."""
    if _points is not None:
        from duckosm import routing
        return routing.route_multimodal(con, src_node, dst_node, schema, start_mode, end_mode, enforce_sequence,
                                        allowed_modes, _points)
    modes = None if allowed_modes is None else list(allowed_modes)
    return json.loads(_rust().route_multimodal(_db(con), int(src_node), int(dst_node), schema, start_mode, end_mode,
                                               enforce_sequence, modes))


def route_points(con, a, b, mode="driving", weight="time", radius_m=50.0, access_kmh=4.5):
    """See :func:`duckosm.point_routing.route_points`."""
    return json.loads(_rust().route_points(_db(con), tuple(a), tuple(b), mode, weight, float(radius_m), float(access_kmh)))


def route_multimodal_points(con, a, b, radius_m=50.0, access_kmh=4.5, schema="mm", allowed_modes=None,
                            enforce_sequence=True):
    """See :func:`duckosm.point_routing.route_multimodal_points`."""
    modes = None if allowed_modes is None else list(allowed_modes)
    return json.loads(_rust().route_multimodal_points(_db(con), tuple(a), tuple(b), float(radius_m), float(access_kmh),
                                                      schema, modes, enforce_sequence))


def directions(con, route, mode="driving"):
    """See :func:`duckosm.directions.directions`."""
    return json.loads(_rust().directions(_db(con), json.dumps(route), mode))


def build_lane_graph(gmns_db, mode="driving"):
    """See :func:`duckosm.lane_routing.build_lane_graph`."""
    lanes, lc, turn = _rust().build_lane_graph(_db(gmns_db), mode)
    return {"lanes": lanes, "lane_change": lc, "turn": turn, "edges": lc + turn}


def route_lanes(gmns_db, from_lane, to_lane, mode="driving"):
    """See :func:`duckosm.lane_routing.route_lanes`."""
    return json.loads(_rust().route_lanes(_db(gmns_db), str(from_lane), str(to_lane), mode))
