"""The checks of a GMNS db: a thin layer on the Rust ``duckosm_export::gmns_check``."""
from duckosm import _api


def check_gmns(path, modes=None):
    """The checks of every ``gmns_<mode>`` schema (or only ``modes``): ``[(schema, check, bad, total), ...]``; bad 0 passes."""
    return _api.check_gmns(path, modes=modes)
