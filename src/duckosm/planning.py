"""``duckosm.planning``: transport planning (docs/proposals/python-sdk.md, section 5.5).

Today: ``to_gmns``, ``check_gmns``, ``to_meso``, ``to_micro``. Later: ``to_aequilibrae``, ``to_path4gmns``.
"""
from duckosm.gmns import to_meso, to_micro  # noqa: F401
from duckosm.gmns_check import check_gmns  # noqa: F401


def to_gmns(net, out_path, mode=None, modes=None, **kwargs):
    """The GMNS network (links, nodes, lanes, movements) in a DuckDB file; ``to_csv=`` also writes the CSV files. A
    ``ModeView`` or ``mode=`` gives one mode; else ``modes`` (default: all modes of the file). Returns the counts.

    >>> duckosm.planning.to_gmns(net, "out/gmns.duckdb", to_csv="out/gmns")"""
    from duckosm.gmns import to_gmns as gmns
    if hasattr(net, "network"):                       # a ModeView
        net, modes = net.network, [net.mode]
    elif mode is not None:
        modes = [mode]
    return gmns(net, out_path, modes=modes, **kwargs)
