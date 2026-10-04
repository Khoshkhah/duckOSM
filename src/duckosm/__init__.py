"""
duckOSM - High-performance OSM-to-routing-network converter built on DuckDB.
"""

from duckosm.importer import DuckOSM
from duckosm.config import Config
from duckosm.edge_id import (edge_id_hash, edge_id_expr, edge_id_hash_v1, edge_id_expr_v1,
                             create_edge_id_macro)
from duckosm.routing import (to_networkx, to_networkx_nodes, write_graph, route, Router,
                             route_multimodal)
from duckosm.point_routing import route_points, route_multimodal_points
from duckosm.directions import directions
from duckosm.sumo import to_sumo, DEFAULT_NETCFG
from duckosm.gis import to_gis
from duckosm.gmns import to_gmns, to_meso, to_micro
from duckosm.gmns_check import check_gmns
from duckosm.matsim import to_matsim
from duckosm.matsim_lanes import to_matsim_lanes
from duckosm.opendrive import to_opendrive
from duckosm.railml import to_railml
from duckosm.lanelet2 import to_lanelet2
from duckosm.lane_routing import build_lane_graph, route_lanes
from duckosm.elevation import to_elevation

__version__ = "0.2.0"
__all__ = ["DuckOSM", "Config", "edge_id_hash", "edge_id_expr", "edge_id_hash_v1",
           "edge_id_expr_v1", "create_edge_id_macro",
           "to_networkx", "to_networkx_nodes", "write_graph", "route", "Router",
           "route_multimodal", "route_points", "route_multimodal_points", "directions", "to_sumo", "DEFAULT_NETCFG", "to_gis", "to_gmns", "to_meso",
           "to_micro", "check_gmns", "to_matsim", "to_matsim_lanes", "to_opendrive", "to_railml", "to_lanelet2",
           "build_lane_graph", "route_lanes", "to_elevation"]
