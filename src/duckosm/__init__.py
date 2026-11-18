"""
duckOSM - High-performance OSM-to-routing-network converter built on DuckDB.

The names below are loaded when they are first used, so that ``import duckosm`` (and the program ``duckosm``) start
fast. The exports, the routing and the elevation call the Rust code (``duckosm._api``, docs/design/all_in_rust.md).
"""

__version__ = "0.7.0"

_NAMES = {
    "DuckOSM": "duckosm.importer", "Config": "duckosm.config",
    "edge_id_hash": "duckosm.edge_id", "edge_id_expr": "duckosm.edge_id", "edge_id_hash_v1": "duckosm.edge_id",
    "edge_id_expr_v1": "duckosm.edge_id", "create_edge_id_macro": "duckosm.edge_id",
    "to_networkx": "duckosm.routing", "to_networkx_nodes": "duckosm.routing", "write_graph": "duckosm.routing",
    "route": "duckosm._api", "Router": "duckosm._api", "route_multimodal": "duckosm._api",
    "route_points": "duckosm._api", "route_multimodal_points": "duckosm._api", "directions": "duckosm._api",
    "to_sumo": "duckosm._api", "DEFAULT_NETCFG": "duckosm.sumo", "to_gis": "duckosm._api",
    # TODO (step 6): GMNS is still Python, until its Rust PR merges
    "to_gmns": "duckosm.gmns", "to_meso": "duckosm.gmns", "to_micro": "duckosm.gmns", "check_gmns": "duckosm.gmns_check",
    "to_matsim": "duckosm._api", "to_matsim_lanes": "duckosm._api", "to_opendrive": "duckosm._api",
    "to_railml": "duckosm._api", "to_lanelet2": "duckosm._api",
    "build_lane_graph": "duckosm._api", "route_lanes": "duckosm._api", "to_elevation": "duckosm._api",
}
__all__ = list(_NAMES)


def __getattr__(name):
    if name in _NAMES:
        import importlib
        value = getattr(importlib.import_module(_NAMES[name]), name)
        globals()[name] = value
        return value
    raise AttributeError(f"module 'duckosm' has no attribute {name!r}")


def __dir__():
    return sorted(list(globals()) + __all__)
