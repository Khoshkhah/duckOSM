"""``duckosm.sim``: traffic simulation (docs/proposals/python-sdk.md, section 5.4).

``to_sumo``, ``to_matsim``, ``to_matsim_lanes``; ``sumo_net`` (the SUMO network read back with sumolib) and ``edge_map``
(the SUMO or MATSim id of each edge or link and its duckOSM ``edge_id``, to join the results of a simulation back).
Each export keeps the ``edge_id`` of duckOSM as the id of the edge or link. pip extra: ``duckosm[sim]`` (SUMO's
``netconvert`` and sumolib).
"""
from duckosm.network import resolve
from duckosm.sumo import DEFAULT_NETCFG  # noqa: F401


def to_sumo(net, out_dir, mode=None, **kwargs):
    """The SUMO network of one mode in ``out_dir``: ``.nod.xml`` (with the traffic lights and stop signs), ``.edg.xml``,
    ``.con.xml``, ``.add.xml`` (the bus stops) and the netconvert config ``.netccfg``. Run ``netconvert -c <net_name>.netccfg``,
    or give ``run_netconvert=True``: then also ``.net.xml`` and ``.tll.xml`` (the signal programs). Options as
    :func:`duckosm.to_sumo` (``run_netconvert``, ``net_name``, ``config`` ...). Returns the paths.

    >>> duckosm.sim.to_sumo(net.driving, "out/sumo")"""
    from duckosm import _api
    db, mode = resolve(net, mode)
    return _api.to_sumo(db, out_dir, mode=mode, **kwargs)


def to_matsim(net, out_path, mode=None, **kwargs):
    """The MATSim ``network.xml`` (``.gz``) of one mode, or of a list of modes. Returns the numbers.

    >>> duckosm.sim.to_matsim(net.driving, "out/network.xml.gz")"""
    from duckosm import _api
    if isinstance(mode, (list, tuple)):
        db = net.network if hasattr(net, "network") else net
    else:
        db, mode = resolve(net, mode)
    return _api.to_matsim(db, out_path, mode=mode, **kwargs)


def to_matsim_lanes(gmns_db, out_dir=".", mode="driving", signals=True, cycle_s=90):
    """MATSim lanes and signals from a GMNS database (:func:`duckosm.planning.to_gmns`).

    >>> duckosm.sim.to_matsim_lanes("out/gmns.duckdb", "out/matsim")"""
    from duckosm import _api
    return _api.to_matsim_lanes(gmns_db, out_dir, mode=mode, signals=signals, cycle_s=cycle_s)


def sumo_net(path):
    """The SUMO network ``path`` (a ``.net.xml``, or the dict of :func:`to_sumo`) read with sumolib
    (``sumolib.net.readNet``), with the internal edges.

    >>> sn = duckosm.sim.sumo_net(net.driving.to_sumo("out/sumo", run_netconvert=True))"""
    from duckosm.errors import require
    require("sumolib", "duckosm.sim.sumo_net")
    import sumolib
    if isinstance(path, dict):
        path = path["net"]
    return sumolib.net.readNet(str(path), withInternal=True)


def edge_map(path):
    """The id of each edge of a SUMO network (``.net.xml``, ``.edg.xml``) or of each link of a MATSim network
    (``network.xml``, ``.xml.gz``), and the duckOSM ``edge_id`` of it, as a DataFrame with the columns ``id`` (text, the
    id in the file) and ``edge_id`` (int64). duckOSM writes ``edge_id`` as the id. netconvert can change it: ``123#1``
    (a part of edge 123) gives 123; ``123+456`` (edges joined) gives one row for each edge. With ``geometry.remove=true`` (not the default of
    ``to_sumo``), netconvert joins edges and keeps only the id of the first edge, without ``+``: the other edges of the join
    are not in the table. Internal edges of SUMO
    (``:junction_0``) are not in the table. Join the results of a simulation on ``id``, then the network on ``edge_id``.

    >>> m = duckosm.sim.edge_map("out/sumo/network.net.xml")"""
    import gzip
    import re
    import xml.etree.ElementTree as ET

    import pandas as pd
    if isinstance(path, dict):
        path = path.get("net") or path["edg"]
    path = str(path)
    rows = []
    with (gzip.open(path) if path.endswith(".gz") else open(path, "rb")) as f:
        for _, el in ET.iterparse(f):
            if el.tag in ("edge", "link") and el.get("function") != "internal":
                i = el.get("id")
                rows += [(i, int(p)) for p in re.findall(r"(?:^|\+)-?(\d+)", i)]
            el.clear()
    return pd.DataFrame(rows, columns=["id", "edge_id"]).astype({"edge_id": "int64"})
