"""The SUMO export: a thin layer on the Rust export (``rust/duckosm-export/src/sumo.rs``). docs/exports/sumo.md.

    out = to_sumo("monaco.duckdb", "sumo/")      # sumo/network.{nod,edg,con,add}.xml + .netccfg (run_netconvert=True: + .net.xml, .tll.xml)

Every SUMO edge id is the duckOSM ``edge_id``; the connections are the legal turns of ``edge_graph``, so the turn
restrictions are respected. The input is a duckOSM db or a GMNS db (docs/design/sim_from_gmns.md).
"""
from duckosm import _api

# the built-in netconvert options (the same as the Rust export's DEFAULT_NETCFG); config={...} sets others over them
DEFAULT_NETCFG = {
    "proj.utm": "true",                      # node/edge coords are lon/lat: project them to UTM
    "geometry.remove": "false",              # do NOT merge degree-2 ways -> edge_id stays 1:1
    "junctions.join": "false",               # do NOT merge nearby nodes
    "no-turnarounds.except-deadend": "true",  # honour the explicit topology (no invented U-turns)
    "numerical-ids": "false",                # keep our ids verbatim
    "tls.default-type": "actuated",          # OSM has no signal timings: a program that follows the traffic (SUMO's advice for OSM)
}


def to_sumo(con, out_dir, mode="driving", net_name="network", run_netconvert=False, netconvert_bin=None, connections=True,
            config=None, bus_edges=False, edge_attrs=None, lanes=None, merge=False):
    """Export the network of ``mode`` of the database ``con`` (a path, or a connection to a database file) to SUMO in
    ``out_dir``, keeping ``edge_id`` and the turn restrictions.

    ``run_netconvert``: also run netconvert (the ``.net.xml`` and the signal programs ``.tll.xml``); ``netconvert_bin``: the
    program (default: found by itself). ``connections``: write the legal turns (else netconvert guesses them). ``config``: a
    ``.netccfg`` path, or a dict of netconvert options over :data:`DEFAULT_NETCFG`. ``bus_edges``: driving only, also the
    bus-only edges. ``edge_attrs``: ``{edge_id: {sumo-edge-attribute: value}}`` over what duckOSM writes. ``lanes``:
    ``{edge_id: [(use, width[, turn]), ...]}`` left to right, each edge's lanes instead of ``driving.lane_profile``'s.
    ``merge``: driving only, one SUMO edge for each chain of ``driving.chains``.

    Returns the paths and the counts (``nod``, ``edg``, ``con``, ``add``, ``netccfg``, ``net``, ``tll``, ``n_*``)."""
    return _api.to_sumo(con, out_dir, mode=mode, net_name=net_name, run_netconvert=run_netconvert, netconvert_bin=netconvert_bin,
                        connections=connections, config=config, bus_edges=bus_edges, edge_attrs=edge_attrs, lanes=lanes, merge=merge)


def _find_netconvert(explicit=None):
    """The netconvert program: ``explicit``, the eclipse-sumo wheel, ``$SUMO_HOME/bin``, or the PATH. FileNotFoundError when it
    is not found."""
    found = _api.find_netconvert(explicit)
    if not found:
        raise FileNotFoundError("netconvert not found. Install SUMO (system package) or `pip install duckosm[sumo]`, "
                                "or pass netconvert_bin=..., or run with run_netconvert=False to write only the plain XML.")
    return found
