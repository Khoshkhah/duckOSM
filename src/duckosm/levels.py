"""The drawing order of a built network, computed by roadstyle and stored in the file (docs/design/levels.md).

``duckosm levels`` reads the roads of every mode, asks roadstyle for the casing and fill numbers of every road
(``roadstyle.compute_levels``) and writes them to ``visualization.edge_levels`` (``roadstyle.save_levels``). Needs the
``levels`` extra (roadstyle with its solver); roadstyle is imported inside the functions.
"""
from __future__ import annotations

import logging

logger = logging.getLogger(__name__)

BAND_RULE = "tags"             # how the band is made, stored in edge_levels_meta.band_rule: "tags" = the layer, bridge, tunnel only (docs/design/levels.md). A table with none is the older rule
MODES = ("driving", "walking", "cycling")


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
    geometry = gpd.GeoSeries([wkb.loads(bytes(b)) for b in df.pop("wkb")], crs="EPSG:4326")
    return gpd.GeoDataFrame(pd.concat([df, pd.Series(band, name="band")], axis=1), geometry=geometry)


def reverse_rows(db):
    """``{edge_id of a reverse row: edge_id of its road}`` for every edge of ``db`` that is ``is_reverse`` in every table it is in (the mode networks do not all flag it the same way): the road is the row of the same ``osm_id`` with ``source`` and ``target`` swapped
    (the smallest ``edge_id`` if there are several). A reverse row with no road is a ``ValueError`` that says how many and the first ids."""
    import duckdb

    con = duckdb.connect(str(db), read_only=True)
    try:
        have = {r[0] for r in con.execute("SELECT table_schema FROM information_schema.tables WHERE table_name = 'edges'").fetchall()}
        parts = [f"SELECT edge_id, osm_id, source, target, is_reverse FROM {m}.edges" for m in MODES if m in have]
        parts += [f"SELECT edge_id, osm_id, source, target, is_reverse FROM {m}.private_edges" for m in MODES if m in have and con.execute(
            "SELECT count(*) FROM information_schema.tables WHERE table_schema = ? AND table_name = 'private_edges'", [m]).fetchone()[0]]
        rows = con.execute(f"""WITH e AS (SELECT edge_id, any_value(osm_id) AS osm_id, any_value(source) AS source, any_value(target) AS target, bool_and(is_reverse) AS is_reverse
                                         FROM ({' UNION ALL '.join(parts)}) GROUP BY edge_id)
                               SELECT r.edge_id, min(f.edge_id) FROM e r LEFT JOIN e f
                               ON NOT f.is_reverse AND f.osm_id = r.osm_id AND f.source = r.target AND f.target = r.source
                               WHERE r.is_reverse GROUP BY r.edge_id ORDER BY r.edge_id""").fetchall()
    finally:
        con.close()
    lost = [e for e, f in rows if f is None]
    if lost:
        raise ValueError(f"{len(lost)} reverse edge(s) have no road in {db} (same osm_id, source and target swapped; first: {lost[:5]})")
    return {int(e): int(f) for e, f in rows}


def compute_and_store(db, order="class", **options):
    """Compute the numbers of the roads of ``db`` and write them to ``visualization.edge_levels`` / ``edge_levels_meta``. ``order``: ``"class"`` or ``None``;
    ``options``: ``band_dist``, ``head_m``, ``max_level``, ``margin``, ``time_limit`` of ``roadstyle.compute_levels``.
    Returns ``{"roads", "positions", "given_up", "info"}``."""
    import duckdb

    rs = roadstyle_module()
    edges = load_roads(db)                                               # one row for every edge_id, reverse rows too
    twin = reverse_rows(db)                                              # a reverse row has the numbers of its road, its two heads swapped (docs/design/levels.md)
    roads = edges[~edges["edge_id"].isin(twin)].reset_index(drop=True)
    computed = rs.compute_levels(roads, method="solve", band_col="band", order=order, **options)
    cols = ("casing_start", "casing_level", "casing_end", "fill_level")
    by_id = {int(e): k for k, e in enumerate(computed["edge_id"])}
    levels = edges.copy()
    heads = {"casing_start": "casing_end", "casing_end": "casing_start"}      # a reverse row runs the other way: its start head is its road's end head
    for c in cols:
        levels[c] = [computed[heads.get(c, c) if int(e) in twin else c].iloc[by_id[twin.get(int(e), int(e))]] for e in edges["edge_id"]]
    levels.attrs.update(computed.attrs)
    con = duckdb.connect(str(db))                                        # read-write: this writes the visualization schema
    try:
        con.execute("INSTALL spatial; LOAD spatial;")                    # the file has spatial indexes: the checkpoint needs the extension
        rs.save_levels(con, levels)
        con.execute("ALTER TABLE visualization.edge_levels_meta ADD COLUMN IF NOT EXISTS band_rule VARCHAR")           # roadstyle's table has none: the rule of the band the numbers were solved from
        con.execute("UPDATE visualization.edge_levels_meta SET band_rule = ?", [BAND_RULE])
        con.execute("CHECKPOINT")
    finally:
        con.close()
    positions = len({int(v) for c in cols for v in levels[c]})
    return {"roads": len(roads), "positions": positions, "given_up": len(levels.attrs["levels_given_up"]), "info": levels.attrs["levels_info"]}
