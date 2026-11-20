"""``duckosm.planning``: transport planning (docs/proposals/python-sdk.md, section 5.5).

``to_gmns``, ``check_gmns``, ``to_meso``, ``to_micro``; ``to_path4gmns`` (the GMNS network loaded in path4gmns) and
``to_aequilibrae`` (an AequilibraE project with the links and nodes). pip extra: ``duckosm[planning]``.
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


def _gmns_csv(gmns, out_dir, mode):
    """Write ``node.csv`` and ``link.csv`` of the GMNS schema ``gmns_<mode>`` of the database ``gmns`` in ``out_dir``
    (DuckDB writes them). Returns ``out_dir``."""
    import duckdb
    from pathlib import Path
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    con = duckdb.connect(str(gmns), read_only=True)
    try:
        for table in ("node", "link"):
            cols = [c for c, in con.execute(
                "SELECT column_name FROM duckdb_columns() WHERE schema_name = ? AND table_name = ? AND data_type <> 'GEOMETRY'",
                [f"gmns_{mode}", table]).fetchall()]
            if not cols:
                raise ValueError(f"{gmns}: no table gmns_{mode}.{table}. Make it with duckosm.planning.to_gmns(...)")
            sel = ", ".join(f'"{c}"' for c in cols)
            path = str(out / f"{table}.csv").replace("'", "''")
            con.execute(f"COPY (SELECT {sel} FROM gmns_{mode}.{table}) TO '{path}' (HEADER)")
    finally:
        con.close()
    return out


def to_path4gmns(gmns, out_dir=None, mode="driving"):
    """The GMNS network of one mode loaded in path4gmns (``path4gmns.read_network``), for its shortest paths, traffic
    assignment and DTALite. ``gmns``: the GMNS database of :func:`to_gmns`. path4gmns reads ``node.csv`` and
    ``link.csv`` as duckOSM writes them, with no change: each node is in the zone 1, ``length`` is in metres and
    ``free_speed`` in km/h. ``out_dir``: the folder of the two CSV files (default: a ``path4gmns`` folder beside the
    database). Returns the path4gmns network.

    >>> pnet = duckosm.planning.to_path4gmns("out/gmns.duckdb")
    >>> pnet.find_shortest_path(node_a, node_b, mode="auto", seq_type="link")"""
    from pathlib import Path

    from duckosm.errors import require
    pg = require("path4gmns", "duckosm.planning.to_path4gmns")
    import warnings
    out = _gmns_csv(gmns, out_dir or Path(gmns).with_name("path4gmns"), mode)
    with warnings.catch_warnings():     # "Please provide settings.yml": its defaults (one period, the agent auto) apply
        warnings.filterwarnings("ignore", message="Please provide settings.yml")
        return pg.read_network(length_unit="meter", speed_unit="kmh", input_dir=str(out))


_AEQ_MODES = {"driving": "c", "walking": "w", "cycling": "b"}


def to_aequilibrae(net, project_dir, mode=None):
    """A new AequilibraE project in ``project_dir`` (the folder must not exist) with the network of one mode: one link
    for each edge (``direction`` 1, from ``source`` to ``target``), with ``name``, ``speed_ab`` (km/h), ``capacity_ab``
    (lanes × the capacity of a lane of the road class, as in GMNS), the mode (``c`` car, ``w`` walk, ``b`` bicycle),
    the ``link_type`` (the OSM ``highway``) and the field ``edge_id``. AequilibraE makes the nodes from the ends of the
    links. Needs aequilibrae and SpatiaLite (``mod_spatialite``). Returns ``{path, links, nodes}``.

    >>> duckosm.planning.to_aequilibrae(net.driving, "out/aequilibrae")"""
    from pathlib import Path

    from duckosm.errors import require
    from duckosm.gmns import _capacity_case
    from duckosm.network import Network, resolve
    require("aequilibrae", "duckosm.planning.to_aequilibrae")
    from aequilibrae import Project
    db, mode = resolve(net, mode)
    db = db if isinstance(db, Network) else Network(db)
    hw = "coalesce(regexp_replace(split_part(highway, ';', 1), '_link$', ''), 'road')"
    rows = db.sql(f"""SELECT row_number() OVER (ORDER BY edge_id) AS link_id, edge_id, coalesce(highway, 'road') AS link_type,
                             name, maxspeed_kmh::DOUBLE AS speed, coalesce(lanes, 1) * ({_capacity_case(hw)})::DOUBLE AS capacity,
                             cost_s::DOUBLE / 60 AS minutes, ST_AsWKB(geometry) AS wkb
                      FROM "{mode}".edges ORDER BY edge_id""")
    path = Path(project_dir)
    project = Project()
    project.new(str(path))
    try:
        project.network.links.fields.add("edge_id", "the duckOSM edge_id of the link", "INTEGER")
        # one link type for each OSM highway value; AequilibraE gives each a one-letter id
        letters = [c for c in "abcdefghijklmnopqrstuvwxABCDEFGHIJKLMNOPQRSTUVWXYZ" if c not in ("y", "z")]
        types = sorted(set(rows.link_type))
        if len(types) > len(letters):
            raise ValueError(f"{len(types)} highway values: AequilibraE has {len(letters)} link type ids")
        m = _AEQ_MODES.get(mode, "c")
        with project.db_connection as con:
            con.executemany("INSERT INTO link_types (link_type, link_type_id, description) VALUES (?, ?, ?)",
                            [(t, letters[i], f"OSM highway={t}") for i, t in enumerate(types)])
            con.executemany(
                "INSERT INTO links (link_id, a_node, b_node, direction, modes, link_type, name, speed_ab, capacity_ab, "
                "travel_time_ab, edge_id, geometry) VALUES (?, 0, 0, 1, ?, ?, ?, ?, ?, ?, ?, GeomFromWKB(?, 4326))",
                [(int(r.link_id), m, r.link_type, r.name, r.speed, r.capacity, r.minutes, int(r.edge_id), bytes(r.wkb))
                 for r in rows.itertuples()])
        with project.db_connection as con:
            links, nodes = (con.execute(f"SELECT count(*) FROM {t}").fetchone()[0] for t in ("links", "nodes"))
    finally:
        project.close()
    return {"path": path, "links": links, "nodes": nodes}
