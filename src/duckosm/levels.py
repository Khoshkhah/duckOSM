"""The drawing order of a built network: its level area, made and solved by roadstyle, the result stored in the file (docs/design/levels.md).

``duckosm levels DB`` puts the roads of every mode in one level area folder (``DB.levels``: roadstyle's roads.parquet, pairs.csv and
your edits.csv, heads.csv, caps.csv), solves it and writes the result to ``visualization.edge_levels``. ``roadstyle-levels edit DB.levels``
is the editor; each of its solves writes into the file too, with your edits.csv, heads.csv and caps.csv (``visualization.level_*``), so a
new area folder of the file gets them back. Needs the ``levels`` extra; roadstyle is imported inside the functions.
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
    def walked(m, t):                                    # a cycling edge where bikes are pushed (dismount): walked, not ridden
        return "dismount" if (m, t, "dismount") in has else "NULL"
    parts = [f"SELECT {cols}, {optional(m, 'edges')}, {ow(m, 'edges')} AS oneway, {walked(m, 'edges')} AS dismount, '{m}' AS mode, NULL AS pmode, {i} AS rank FROM {m}.edges"
             for i, m in enumerate(modes)]
    parts += [f"SELECT {cols}, {optional(m, 'private_edges')}, {ow(m, 'private_edges')} AS oneway, {walked(m, 'private_edges')} AS dismount, NULL AS mode, '{m}' AS pmode, "
              f"{len(MODES) + i} AS rank FROM {m}.private_edges"
              for i, m in enumerate(modes) if m in private]
    return " UNION ALL ".join(parts)


def _line_key(ref):
    """Bus line refs in natural order: 2 before 11, numbers before letters."""
    return (not ref.isdigit(), int(ref) if ref.isdigit() else 0, ref)


def load_roads(db):
    """The roads of all modes in ``db``: one row per ``edge_id`` (in ``edge_id`` order) with ``highway``, ``layer``, ``bridge``, ``tunnel``,
    ``walk_type``, ``junction``, ``name``, ``edge_ref``, ``lanes`` (NULL where a table of an older file has none), ``driving`` / ``cycling`` (in that mode's network, private edges not), ``oneway``
    (the DRIVING network's: roadstyle's arrows are cars' one-ways only; NULL off it), ``modes`` (the modes whose
    network has it, e.g. ``driving + walking``; cycling only where bikes are ridden: where they are pushed (``dismount``) it is walking (2026-10-09), and ``bus`` where a bus route runs on it; a private road only: its modes with "(private)", e.g. ``walking (private)``), ``bus_lines`` (the refs of the bus
    routes on it, e.g. ``1, 2, 5``, from ``bus.route_edges``; None off them or without that table), the band ``band`` and the line. The attributes are those of the first row, the modes taken in the order driving, walking,
    cycling and ``edges`` before ``private_edges``."""
    import duckdb
    import geopandas as gpd
    import numpy as np
    import pandas as pd
    from shapely import wkb

    from duckosm.crossings import _level

    def walk(c):        # a cycling edge where bikes are pushed (dismount) is walked: walking, not cycling (2026-10-09)
        return f"CASE WHEN {c} = 'cycling' AND coalesce(dismount, false) THEN 'walking' ELSE {c} END"

    con = duckdb.connect(str(db), read_only=True)
    try:
        con.execute("INSTALL spatial; LOAD spatial;")
        df = con.execute(f"""
            SELECT edge_id, first(highway ORDER BY rank) AS highway, first(layer ORDER BY rank) AS layer,
                   first(bridge ORDER BY rank) AS bridge, first(tunnel ORDER BY rank) AS tunnel,
                   {", ".join(f"first({c} ORDER BY rank) AS {c}" for c in OPTIONAL)}, list(DISTINCT {walk('mode')}) FILTER (WHERE mode IS NOT NULL) AS mode_list,
                   list(DISTINCT {walk('pmode')}) FILTER (WHERE pmode IS NOT NULL) AS private_list,
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
    have = lambda ms: set(ms if isinstance(ms, (list, np.ndarray)) else [])      # noqa: E731
    # who may use it: the editor shows it, lanestyle colours a road cars do not use by it; a private road only: its modes "(private)" (2026-10-09:
    # a private footway was None, so nothing said it is walked)
    df["modes"] = [" + ".join(([m for m in MODES if m in have(ms)] or [f"{m} (private)" for m in MODES if m in have(ps)]) + (["bus"] if isinstance(bl, str) else [])) or None
                   for ms, ps, bl in zip(df.pop("mode_list"), df.pop("private_list"), df["bus_lines"], strict=True)]
    geometry = gpd.GeoSeries([wkb.loads(bytes(b)) for b in df.pop("wkb")], crs="EPSG:4326")
    return gpd.GeoDataFrame(pd.concat([df, pd.Series(band, name="band")], axis=1), geometry=geometry)


# the level area's own input, kept in the database (roadstyle: visualization.level_*): {csv file: (table, the columns that name roads)}
INPUTS = {"edits.csv": ("level_edits", ("a", "b")), "heads.csv": ("level_heads", ("road",)), "caps.csv": ("level_caps", ("road",))}


def read_inputs(db):
    """The level inputs kept in the database file ``db``: ``{table: DataFrame of text}`` for ``level_edits``, ``level_heads``, ``level_caps`` and
    ``level_unmatched`` (docs/design/levels_in_build.md); ``{}`` when the file or the tables are not there. Every value is text, so a 64-bit
    ``edge_id`` is not rounded."""
    import duckdb

    if not Path(db).exists():
        return {}
    try:
        con = duckdb.connect(str(db), read_only=True)
    except duckdb.Error:
        return {}
    out = {}
    try:
        have = {r[0] for r in con.execute("SELECT table_name FROM information_schema.tables WHERE table_schema = 'visualization'").fetchall()}
        for table in [t for t, _ in INPUTS.values()] + ["level_unmatched"]:
            if table in have:
                cols = [r[0] for r in con.execute(f"DESCRIBE visualization.{table}").fetchall()]
                out[table] = con.execute("SELECT " + ", ".join(f'CAST("{c}" AS VARCHAR) AS "{c}"' for c in cols) + f" FROM visualization.{table}").df()
    finally:
        con.close()
    return out


def split_inputs(inputs, ids):
    """``(kept, unmatched)``: ``kept`` is ``{csv file: DataFrame}`` of the rows whose roads are all in ``ids`` (a set of ``edge_id`` text);
    ``unmatched`` is a DataFrame (``kind`` = the table, ``row`` = the row as JSON) of the others, with the earlier unmatched rows tried again.
    A table the inputs do not have is not in ``kept``."""
    import json

    import pandas as pd

    tables = {t: df.copy() for t, df in inputs.items() if t != "level_unmatched"}
    for _, r in inputs.get("level_unmatched", pd.DataFrame(columns=["kind", "row"])).iterrows():
        row = json.loads(r["row"])
        if r["kind"] in tables:
            tables[r["kind"]] = pd.concat([tables[r["kind"]], pd.DataFrame([row])], ignore_index=True)
    kept, unmatched = {}, []
    for f, (table, id_cols) in INPUTS.items():
        if table not in tables:
            continue
        df = tables[table]
        ok = pd.Series(True, index=df.index)
        for c in id_cols:
            if c in df.columns:
                ok &= df[c].isna() | df[c].isin(ids)
        kept[f] = df[ok]
        unmatched += [{"kind": table, "row": json.dumps({k: (None if pd.isna(v) else v) for k, v in r.items()})} for _, r in df[~ok].iterrows()]
    return kept, pd.DataFrame(unmatched, columns=["kind", "row"])


def sync_inputs(db, area, roads, inputs=None, keep_unmatched=True):
    """Write the folder's edits.csv / heads.csv / caps.csv from the tables of ``db`` (or ``inputs``, see :func:`read_inputs`): the tables are the
    master (docs/design/levels_in_build.md). The rows of roads that are not in ``roads`` are left out; returns them as a DataFrame (``kind``, ``row``),
    or ``None`` when roadstyle has no ``level_*`` tables (the folder's files are then the master, as before). A folder file that differs is kept
    as ``*.csv.before``. ``keep_unmatched=False``: a clip's rows from its parent for the roads outside the clip are not kept."""
    rs = roadstyle_module()
    area = Path(area)
    if not hasattr(rs, "restore_area_inputs"):
        logger.warning("levels: this roadstyle does not keep your edits in the database (no level_* tables): the folder's files are used")
        return None
    inputs = read_inputs(db) if inputs is None else inputs
    kept, unmatched = split_inputs(inputs, set(roads["edge_id"].astype(str)))
    if not keep_unmatched:
        unmatched = unmatched.iloc[0:0]
    area.mkdir(parents=True, exist_ok=True)
    for f, df in kept.items():
        path = area / f
        if path.exists() and path.read_text() != df.to_csv(index=False):
            path.with_name(path.name + ".before").write_text(path.read_text())          # the folder's own, if it differs: not lost
        df.to_csv(path, index=False)
    if len(unmatched):
        logger.warning(f"levels: {len(unmatched)} edit row(s) name a road that is not in this file: ignored, kept in visualization.level_unmatched")
    return unmatched


def is_current(db, area, roads=None):
    """True when ``area`` holds a solved level area of exactly the roads in ``db`` (the stored numbers belong to them): no new solve is needed."""
    import duckdb

    rs = roadstyle_module()
    area = Path(area)
    if not ((area / "levels.csv").exists() and (area / "area.json").exists()):
        return False
    con = duckdb.connect(str(db), read_only=True)
    try:
        rs.load_area_levels(con, load_roads(db) if roads is None else roads)
        return True
    except ValueError:
        return False
    finally:
        con.close()


def make_and_solve(db, area=None, inputs=None, keep_unmatched=True):
    """The level area of ``db`` (docs/design/levels.md): the roads of every mode in one area folder (``area``, default ``<db>.levels`` next to
    it), solved with your edits, heads and caps, the result written into ``db`` (``visualization.edge_levels`` with each
    edge's ends). roadstyle does all of it (``roadstyle.level_area``); the area belongs to ``db``, so ``roadstyle-levels edit AREA``
    writes into it too. Returns ``{"area", "roads", "given_up", "info", "unmatched"}``.

    The tables are the master (:func:`sync_inputs`): ``inputs`` (default: the ones in ``db``) are written to the folder's files first."""
    roadstyle_module()
    from roadstyle.level_area import make_area, solve_area

    area = Path(area) if area else Path(db).with_suffix(".levels")
    roads = load_roads(db)
    unmatched = sync_inputs(db, area, roads, inputs, keep_unmatched)
    make_area(roads, area, db=db, id_col="edge_id")
    solved = solve_area(area)
    if unmatched is not None:
        import duckdb

        con = duckdb.connect(str(db))
        try:
            con.execute("CREATE SCHEMA IF NOT EXISTS visualization")
            if len(unmatched):
                con.register("_unmatched", unmatched)
                con.execute("CREATE OR REPLACE TABLE visualization.level_unmatched AS SELECT * FROM _unmatched")
                con.unregister("_unmatched")
            else:
                con.execute("DROP TABLE IF EXISTS visualization.level_unmatched")
        finally:
            con.close()
    return {"area": area, "roads": len(solved), "given_up": len(solved.attrs["levels_given_up"]), "info": solved.attrs["levels_info"],
            "unmatched": 0 if unmatched is None else len(unmatched)}
