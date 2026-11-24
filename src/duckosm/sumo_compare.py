"""SUMO from duckOSM's lane profile against SUMO's own OSM import (docs/design/gmns_lane_profile.md, decision 2): a thin layer on
the Rust ``duckosm_export::sumo_compare``."""
from duckosm import _api


def lane_diff(db, osm_file, work_dir, netconvert_bin=None):
    """The differences (a list of dicts: ``way``, ``dir``, ``duckosm``, ``sumo_osm``, ``sources``, ``cause``) and the number of road
    way-directions compared, as ``(rows, compared)``. ``osm_file``: the area's ``.osm`` or ``.osm.pbf``."""
    return _api.sumo_lane_diff(db, osm_file, work_dir, netconvert_bin=netconvert_bin)
