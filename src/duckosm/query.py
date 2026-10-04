"""Cross-mode inspection queries — everything a duckOSM db knows about one ``osm_id``.

One raw OSM way is extracted into per-mode edge tables (``driving.edges`` / ``walking.edges`` /
``cycling.edges``), whose schemas differ (driving carries ``maxspeed_kmh``/``cost_s``, walking
``walk_type``, cycling ``cycle_type``, …). :func:`way_table` unions them back into ONE long-format
table with a leading ``mode`` column — the union of all per-mode columns, NULL-filled where a mode
lacks one — so a way's segmentation and attributes can be compared across modes side by side
(same ``edge_id``/``edge_ref`` ⇒ the cross-mode alignment invariant holds). Also works for
synthetic ids: pass a negative ``osm_id`` to inspect PathConnector connector edges.

Used by ``duckosm way <db> <osm_id>`` (see cli.py); importable for notebooks:

    import duckdb
    from duckosm.query import way_table
    con = duckdb.connect("data/db/tartu.duckdb", read_only=True)
    way_table(con, 4934235).show()
"""
MODES = ("driving", "walking", "cycling")

# never useful in a tabular report unless asked for (rendered as WKT via --geom)
_GEOM_COLS = {"geometry"}


def _mode_schemas(con, modes=None):
    """The mode schemas that actually exist in this db (optionally restricted)."""
    have = {r[0] for r in con.execute(
        "SELECT DISTINCT table_schema FROM information_schema.tables "
        "WHERE table_catalog = current_database() "      # not an attached parent's modes
        "AND table_name = 'edges'").fetchall()}
    return [m for m in (modes or MODES) if m in have]


def _edge_columns(con, mode):
    return [r[0] for r in con.execute(
        "SELECT column_name FROM information_schema.columns "
        "WHERE table_schema = ? AND table_name = 'edges' ORDER BY ordinal_position",
        [mode]).fetchall()]


def way_table_sql(con, osm_id, modes=None, geometry=False):
    """Build the UNION query for :func:`way_table` (kept separate for reuse/EXPLAIN)."""
    schemas = _mode_schemas(con, modes)
    if not schemas:
        raise ValueError("no mode schemas with an edges table in this database")
    # column union across modes, in first-seen order, so every SELECT arm lines up
    cols, per_mode = [], {}
    for m in schemas:
        per_mode[m] = set(_edge_columns(con, m))
        for c in _edge_columns(con, m):
            if c not in cols and not (c in _GEOM_COLS and not geometry):
                cols.append(c)
    arms = []
    for m in schemas:
        sel = []
        for c in cols:
            if c not in per_mode[m]:
                sel.append(f'NULL AS "{c}"')
            elif c in _GEOM_COLS:
                sel.append(f'ST_AsText("{c}") AS "{c}"')
            else:
                sel.append(f'"{c}"')
        arms.append(f"SELECT '{m}' AS mode, {', '.join(sel)}\n"
                    f"FROM {m}.edges WHERE osm_id = {int(osm_id)}")
    union = "\nUNION ALL\n".join(arms)
    # traversal order (edge_ref seq), forward before reverse, then mode — NULL-safe for
    # pre-edge_ref builds
    order = ("COALESCE(TRY_CAST(regexp_extract(edge_ref, '#(\\d+)', 1) AS INT), 2000000000), "
             "is_reverse, mode" if any("edge_ref" in per_mode[m] for m in schemas)
             else "source, target, is_reverse, mode")
    return f"SELECT * FROM (\n{union}\n) ORDER BY {order}"


def way_table(con, osm_id, modes=None, geometry=False):
    """All edges extracted from one ``osm_id``, across all modes, as ONE relation."""
    return con.sql(way_table_sql(con, osm_id, modes=modes, geometry=geometry))


def way_raw(con, osm_id):
    """The raw OSM way row (tags + refs), or None (synthetic osm_id / clip build w/o raw)."""
    has_raw = con.execute(
        "SELECT count(*) FROM information_schema.tables "
        "WHERE table_catalog = current_database() "      # a clip has no raw schema of its own
        "AND table_schema = 'raw' AND table_name = 'ways'").fetchone()[0]
    if not has_raw:
        return None
    row = con.execute(
        "SELECT osm_id, refs, tags FROM raw.ways WHERE osm_id = ?", [int(osm_id)]).fetchone()
    if row is None:
        return None
    return {"osm_id": row[0], "refs": row[1], "tags": row[2]}


def db_info(con):
    """What a built database holds (``duckosm info``): per mode its edges, nodes, private edges, km,
    legal turns and turn restrictions, then the build metadata and the other schemas. Read-only."""
    tables = {(s_, t) for s_, t in con.execute(
        "SELECT table_schema, table_name FROM information_schema.tables "
        "WHERE table_catalog = current_database()").fetchall()}
    count = lambda sch, t: (con.execute(f"SELECT count(*) FROM {sch}.{t}").fetchone()[0]
                            if (sch, t) in tables else None)
    modes = []
    for m in _mode_schemas(con):
        edges, km = con.execute(f"SELECT count(*), round(coalesce(sum(length_m), 0) / 1000, 1) "
                                f"FROM {m}.edges").fetchone()
        modes.append({"mode": m, "edges": edges, "nodes": count(m, "nodes"),
                      "private_edges": count(m, "private_edges"), "km": km,
                      "edge_graph": count(m, "edge_graph"),
                      "turn_restrictions": count(m, "turn_restrictions")})
    meta = {}
    if ("main", "visualization_metadata") in tables:
        cols = [r[0] for r in con.execute("DESCRIBE main.visualization_metadata").fetchall()]
        want = [c for c in ("timezone", "built_at", "duckosm_version") if c in cols]
        if want:
            row = con.execute(f"SELECT {', '.join(want)} FROM main.visualization_metadata LIMIT 1").fetchone()
            meta = {c: (str(v) if v is not None else None) for c, v in zip(want, row or [])}
    schemas = {s_ for s_, _ in tables}
    return {"modes": modes,
            "timezone": meta.get("timezone"), "built_at": meta.get("built_at"),
            "duckosm_version": meta.get("duckosm_version"),
            "raw": ("raw", "ways") in tables,
            "features": sorted(t for s_, t in tables if s_ == "features"),
            "multimodal": "mm" in schemas,
            "boundary": ("main", "boundary") in tables,
            "admin_boundaries": count("main", "admin_boundaries") or 0,
            "elevation": ("main", "elevation_metadata") in tables,
            "visualization": ("visualization", "edge_levels") in tables}

