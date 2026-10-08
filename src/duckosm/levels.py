"""The drawing order of a built network: its level area, made and solved by roadstyle, the result stored in the file (docs/design/levels.md).

``duckosm levels DB`` puts the roads of every mode in one level area folder (``DB.levels``: roadstyle's roads.parquet, pairs.csv and
your edits.csv, heads.csv, caps.csv), solves it and writes the result to ``visualization.edge_levels``. ``roadstyle-levels edit DB.levels``
is the editor; each of its solves writes into the file too. Needs the ``levels`` extra; roadstyle is imported inside the functions.
"""
from __future__ import annotations

import logging
from pathlib import Path

logger = logging.getLogger(__name__)

MODES = ("driving", "walking", "cycling")
OPTIONAL = ("walk_type", "junction", "name", "edge_ref", "lanes")   # roadstyle reads junction (a roundabout is on top where roads meet); the editor shows name, edge_ref, lanes


def roadstyle_module():
    """roadstyle with its level areas, or an ImportError that says how to install it."""
    try:
        import roadstyle
        import roadstyle.level_area  # noqa: F401  (roadstyle before 0.17 has none)
    except ImportError as e:
        raise ImportError('the levels need roadstyle 0.17 or later: pip install "duckosm[levels]"') from e
    return roadstyle


def _union(con):
    """The SQL that stacks the ``edges`` and ``private_edges`` of every mode in the file, with the mode and ``walk_type``."""
    have = {r[0] for r in con.execute("SELECT table_schema FROM information_schema.tables WHERE table_name = 'edges'").fetchall()}
    modes = [m for m in MODES if m in have]
    if not modes:
        raise ValueError(f"no <mode>.edges table in this file (modes: {', '.join(MODES)})")
    has = {(s, t, c) for s, t, c in con.execute("SELECT table_schema, table_name, column_name FROM information_schema.columns").fetchall()}
    private = {r[0] for r in con.execute("SELECT table_schema FROM information_schema.tables WHERE table_name = 'private_edges'").fetchall()} & set(modes)
    cols = "edge_id, highway, layer, bridge, tunnel, geometry"

    def optional(m, t):                                  # a column an older file may not have: NULL there
        return ", ".join(f"{c if (m, t, c) in has else 'NULL'} AS {c}" for c in OPTIONAL)
    def ow(m, t):                                        # one-way as the mode's own network says (the driving one decides the arrows)
        return "oneway" if (m, t, "oneway") in has else "NULL"
    parts = [f"SELECT {cols}, {optional(m, 'edges')}, {ow(m, 'edges')} AS oneway, '{m}' AS mode, {i} AS rank FROM {m}.edges" for i, m in enumerate(modes)]
    parts += [f"SELECT {cols}, {optional(m, 'private_edges')}, {ow(m, 'private_edges')} AS oneway, NULL AS mode, {len(MODES) + i} AS rank FROM {m}.private_edges"
              for i, m in enumerate(modes) if m in private]
    return " UNION ALL ".join(parts)


def _line_key(ref):
    """Bus line refs in natural order: 2 before 11, numbers before letters."""
    return (not ref.isdigit(), int(ref) if ref.isdigit() else 0, ref)


def load_roads(db):
    """The roads of all modes in ``db``: one row per ``edge_id`` (in ``edge_id`` order) with ``highway``, ``layer``, ``bridge``, ``tunnel``,
    ``walk_type``, ``junction``, ``name``, ``edge_ref``, ``lanes`` (NULL where a table of an older file has none), ``driving`` / ``cycling`` (in that mode's network, private edges not), ``oneway``
    (the DRIVING network's: roadstyle's arrows are cars' one-ways only; NULL off it), ``modes`` (the modes whose
    network has it, e.g. ``driving + walking``, and ``bus`` where a bus route runs on it; None for a private road only), ``bus_lines`` (the refs of the bus
    routes on it, e.g. ``1, 2, 5``, from ``bus.route_edges``; None off them or without that table), the band ``band`` and the line. The attributes are those of the first row, the modes taken in the order driving, walking,
    cycling and ``edges`` before ``private_edges``."""
    import duckdb
    import geopandas as gpd
    import numpy as np
    import pandas as pd
    from shapely import wkb

    from duckosm.crossings import _level

    con = duckdb.connect(str(db), read_only=True)
    try:
        con.execute("INSTALL spatial; LOAD spatial;")
        df = con.execute(f"""
            SELECT edge_id, first(highway ORDER BY rank) AS highway, first(layer ORDER BY rank) AS layer,
                   first(bridge ORDER BY rank) AS bridge, first(tunnel ORDER BY rank) AS tunnel,
                   {", ".join(f"first({c} ORDER BY rank) AS {c}" for c in OPTIONAL)}, list(DISTINCT mode) FILTER (WHERE mode IS NOT NULL) AS mode_list,
                   coalesce(bool_or(mode = 'driving'), false) AS driving, coalesce(bool_or(mode = 'cycling'), false) AS cycling,
                   bool_or(oneway) FILTER (WHERE mode = 'driving') AS oneway,
                   ST_AsWKB(first(geometry ORDER BY rank)) AS wkb
            FROM ({_union(con)}) GROUP BY edge_id ORDER BY edge_id""").df()
        has_bus = con.execute("SELECT count(*) FROM information_schema.tables WHERE table_schema = 'bus' AND table_name = 'route_edges'").fetchone()[0]
        lines = dict(con.execute("SELECT edge_id, list(DISTINCT coalesce(ref, route_id::VARCHAR)) FROM bus.route_edges GROUP BY edge_id").fetchall()) if has_bus else {}
    finally:
        con.close()
    band = np.array([_level(ly, br, tn) for ly, br, tn in zip(df["layer"], df["bridge"], df["tunnel"], strict=True)], dtype=int)
    df["bus_lines"] = [", ".join(sorted(lines[e], key=_line_key)) if e in lines else None for e in df["edge_id"]]   # the bus lines on it (bus.route_edges), e.g. "1, 2, 5"
    df["modes"] = [" + ".join([m for m in MODES if m in set(ms if isinstance(ms, (list, np.ndarray)) else [])] + (["bus"] if bl else [])) or None
                   for ms, bl in zip(df.pop("mode_list"), df["bus_lines"], strict=True)]   # who may use it: the editor shows it; a private road only: None
    geometry = gpd.GeoSeries([wkb.loads(bytes(b)) for b in df.pop("wkb")], crs="EPSG:4326")
    return gpd.GeoDataFrame(pd.concat([df, pd.Series(band, name="band")], axis=1), geometry=geometry)


def make_and_solve(db, area=None):
    """The level area of ``db`` (docs/design/levels.md): the roads of every mode in one area folder (``area``, default ``<db>.levels`` next to
    it), solved with your edits.csv / heads.csv / caps.csv there, the result written into ``db`` (``visualization.edge_levels`` with each
    edge's ends). roadstyle does all of it (``roadstyle.level_area``); the area belongs to ``db``, so ``roadstyle-levels edit AREA``
    writes into it too. Returns ``{"area", "roads", "given_up", "info"}``."""
    roadstyle_module()
    from roadstyle.level_area import make_area, solve_area

    area = Path(area) if area else Path(db).with_suffix(".levels")
    make_area(load_roads(db), area, db=db, id_col="edge_id")
    solved = solve_area(area)
    return {"area": area, "roads": len(solved), "given_up": len(solved.attrs["levels_given_up"]), "info": solved.attrs["levels_info"]}
