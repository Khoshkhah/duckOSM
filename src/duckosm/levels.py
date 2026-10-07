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
    with_wt = {r[0] for r in con.execute("SELECT table_schema FROM information_schema.columns "
                                         "WHERE table_name = 'edges' AND column_name = 'walk_type'").fetchall()}
    private = {r[0] for r in con.execute("SELECT table_schema FROM information_schema.tables WHERE table_name = 'private_edges'").fetchall()} & set(modes)
    cols = "edge_id, highway, layer, bridge, tunnel, geometry"
    parts = [f"SELECT {cols}, {'walk_type' if m in with_wt else 'NULL'} AS walk_type, {i} AS rank FROM {m}.edges" for i, m in enumerate(modes)]
    parts += [f"SELECT {cols}, NULL AS walk_type, {len(MODES) + i} AS rank FROM {m}.private_edges" for i, m in enumerate(modes) if m in private]
    return " UNION ALL ".join(parts)


def load_roads(db):
    """The roads of all modes in ``db``: one row per ``edge_id`` (in ``edge_id`` order) with ``highway``, ``layer``, ``bridge``, ``tunnel``,
    ``walk_type``, the band ``band`` and the line. The attributes are those of the first row, the modes taken in the order driving, walking,
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
                   first(walk_type ORDER BY rank) AS walk_type, ST_AsWKB(first(geometry ORDER BY rank)) AS wkb
            FROM ({_union(con)}) GROUP BY edge_id ORDER BY edge_id""").df()
    finally:
        con.close()
    band = np.array([_level(ly, br, tn) for ly, br, tn in zip(df["layer"], df["bridge"], df["tunnel"], strict=True)], dtype=int)
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
