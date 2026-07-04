"""
duckOSM - High-performance OSM-to-routing-network converter built on DuckDB.
"""

from duckosm.importer import DuckOSM
from duckosm.config import Config
from duckosm.edge_id import edge_id_hash, edge_id_expr, create_edge_id_macro
from duckosm.routing import (to_networkx, to_networkx_nodes, write_graph, route, Router,
                             route_multimodal)
from duckosm.sumo import to_sumo, DEFAULT_NETCFG
from duckosm.gis import to_gis
from duckosm.gmns import to_gmns, to_meso

__version__ = "0.1.0"
__all__ = ["DuckOSM", "Config", "edge_id_hash", "edge_id_expr", "create_edge_id_macro",
           "to_networkx", "to_networkx_nodes", "write_graph", "route", "Router",
           "route_multimodal", "to_sumo", "DEFAULT_NETCFG", "to_gis", "to_gmns", "to_meso"]
