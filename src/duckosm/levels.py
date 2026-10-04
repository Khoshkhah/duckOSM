"""The drawing order of a built network, computed by roadstyle and stored in the file (docs/design/levels.md).

``duckosm levels`` reads the roads of every mode, asks roadstyle for the casing and fill numbers of every road
(``roadstyle.compute_levels``) and writes them to ``visualization.edge_levels`` (``roadstyle.save_levels``). Needs the
``levels`` extra (roadstyle with its solver); roadstyle is imported inside the functions.
"""
from __future__ import annotations

import logging

logger = logging.getLogger(__name__)

MODES = ("driving", "walking", "cycling")
PATH_CLASSES = ("footway", "path", "cycleway", "steps", "pedestrian", "bridleway", "corridor")
_BAND_OF_WALK_TYPE = {"sidewalk": -1, "crossing": 1}                    # a sidewalk under its street, a crossing over it


def roadstyle_module():
    """roadstyle with the levels solver, or an ImportError that says how to install it."""
    try:
        import roadstyle
        roadstyle.compute_levels, roadstyle.save_levels                  # noqa: B018  (roadstyle before 0.13 has neither)
    except (ImportError, AttributeError) as e:
        raise ImportError('the levels need roadstyle 0.13 or later: pip install "duckosm[levels]"') from e
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
    path = df["highway"].isin(PATH_CLASSES) & df["walk_type"].isin(_BAND_OF_WALK_TYPE)
    band = np.where(path, df["walk_type"].map(_BAND_OF_WALK_TYPE).fillna(0).astype(int), band)
    geometry = gpd.GeoSeries([wkb.loads(bytes(b)) for b in df.pop("wkb")], crs="EPSG:4326")
    return gpd.GeoDataFrame(pd.concat([df, pd.Series(band, name="band")], axis=1), geometry=geometry)


def compute_and_store(db, order="class", **options):
    """Compute the numbers of the roads of ``db`` and write them to ``visualization.edge_levels`` / ``edge_levels_meta``. ``order``: ``"class"`` or ``None``;
    ``options``: ``band_dist``, ``head_m``, ``max_level``, ``margin``, ``time_limit`` of ``roadstyle.compute_levels``.
    Returns ``{"roads", "positions", "given_up", "info"}``."""
    import duckdb

    rs = roadstyle_module()
    roads = load_roads(db)
    levels = rs.compute_levels(roads, method="solve", band_col="band", order=order, **options)
    con = duckdb.connect(str(db))                                        # read-write: this writes the visualization schema
    try:
        con.execute("INSTALL spatial; LOAD spatial;")                    # the file has spatial indexes: the checkpoint needs the extension
        rs.save_levels(con, levels)
        con.execute("CHECKPOINT")
    finally:
        con.close()
    cols = ("casing_start", "casing_level", "casing_end", "fill_level")
    positions = len({int(v) for c in cols for v in levels[c]})
    return {"roads": len(roads), "positions": positions, "given_up": len(levels.attrs["levels_given_up"]), "info": levels.attrs["levels_info"]}
