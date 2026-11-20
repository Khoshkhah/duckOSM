"""
duckOSM - High-performance OSM-to-routing-network converter built on DuckDB.

The names below are loaded when they are first used, so that ``import duckosm`` (and the program ``duckosm``) start
fast. The exports, the routing and the elevation call the Rust code (``duckosm._api``, docs/design/all_in_rust.md).

The SDK (docs/proposals/python-sdk.md): the core (``network``, ``open``, ``Network``, ``ModeView``, ``Area``) and one
module for each group of users: ``analysis``, ``gis``, ``routing``, ``sim``, ``planning``, ``mobility``, ``hdmap``,
``ml`` and ``fixes``. A module is imported when it is first used. The old top-level functions stay.

>>> import duckosm
>>> net = duckosm.network("Monaco")
>>> net.map()
"""

__version__ = "0.8.0"

_NAMES = {
    "DuckOSM": "duckosm.importer", "Config": "duckosm.config",
    "edge_id_hash": "duckosm.edge_id", "edge_id_expr": "duckosm.edge_id", "edge_id_hash_v1": "duckosm.edge_id",
    "edge_id_expr_v1": "duckosm.edge_id", "create_edge_id_macro": "duckosm.edge_id",
    "to_networkx": "duckosm._routing_py", "to_networkx_nodes": "duckosm._routing_py", "write_graph": "duckosm._routing_py",
    "route": "duckosm._api", "Router": "duckosm._api", "route_multimodal": "duckosm._api",
    "route_points": "duckosm._api", "route_multimodal_points": "duckosm._api", "directions": "duckosm._api",
    "to_sumo": "duckosm._api", "DEFAULT_NETCFG": "duckosm.sumo", "to_gis": "duckosm._api",
    # TODO (step 6): GMNS is still Python, until its Rust PR merges
    "to_gmns": "duckosm.gmns", "to_meso": "duckosm.gmns", "to_micro": "duckosm.gmns", "check_gmns": "duckosm.gmns_check",
    "to_matsim": "duckosm._api", "to_matsim_lanes": "duckosm._api", "to_opendrive": "duckosm._api",
    "to_railml": "duckosm._api", "to_lanelet2": "duckosm._api",
    "quack": "duckosm._api", "tiles": "duckosm._api", "extract": "duckosm._api", "boundary": "duckosm._api",
    "init_config": "duckosm._api", "build": "duckosm._api", "levels": "duckosm._api",
    "to_rustworkx": "duckosm._api",
    "get": "duckosm._api", "Area": "duckosm._api", "AreaPicker": "duckosm.widget",
    "Network": "duckosm.network", "ModeView": "duckosm.network", "open": "duckosm.network", "network": "duckosm.network",
    "DuckOSMError": "duckosm.errors",
    "build_lane_graph": "duckosm._api", "route_lanes": "duckosm._api", "to_elevation": "duckosm._api",
}
# the modules of the SDK, one for each group of users: imported when first used (duckosm.sim, duckosm.routing ...)
_MODULES = ("analysis", "gis", "routing", "sim", "planning", "mobility", "hdmap", "ml", "fixes")
__all__ = [n for n in _NAMES if n != "open"]    # duckosm.open, but "from duckosm import *" keeps the built-in open


def __getattr__(name):
    if name in _NAMES:
        import importlib
        try:
            module = importlib.import_module(_NAMES[name])
        except ModuleNotFoundError as e:     # an optional dependency (anywidget for AreaPicker ...): say how to install it
            from duckosm.errors import _PIP, MissingDependency
            if e.name not in _PIP:
                raise
            raise MissingDependency(f"duckosm.{name} needs {e.name}: {_PIP[e.name]}") from e
        value = getattr(module, name)
        globals()[name] = value
        return value
    if name in _MODULES:
        import importlib
        return importlib.import_module(f"duckosm.{name}")
    raise AttributeError(f"module 'duckosm' has no attribute {name!r}")


def __dir__():
    return sorted(set(globals()) | set(__all__) | set(_MODULES))
