"""The GMNS viewer page (``duckosm gmns-viz``): a thin layer on the Rust ``duckosm_export::gmns_viewer``."""
from duckosm import _api


def write_viewer(gmns_db, out, mode="driving"):
    """The viewer page of ``mode`` (lanes, meso and micro networks where they are) at ``out``. Returns its path."""
    return str(_api.gmns_viewer(gmns_db, out, mode=mode)["path"])
