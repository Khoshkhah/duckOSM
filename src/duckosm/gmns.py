"""
GMNS extractor — turn a built duckOSM db into a **standalone GMNS DuckDB**: every GMNS table that OSM
can support (including per-lane detail and turn movements), written into one new ``.duckdb`` file with
native geometry so it is self-contained, queryable and renderable — down to individual lanes. Dump to
spec-standard GMNS CSVs on demand.

GMNS (General Modeling Network Specification, Zephyr/Volpe-FHWA, v0.97) is a directed node–link graph
of flat tables; only ``node`` + ``link`` are required, the rest optional. duckOSM edges are already
directed, so each edge is one GMNS ``link`` (``link_id = edge_id``, exact). The line graph
``edge_graph`` — turn restrictions already removed — becomes the ``movement`` table, and OSM lane tags
(``turn:lanes`` / ``width:lanes`` / ``bicycle:lanes`` / ``psv:lanes``) become per-lane rows.

Per mode a ``gmns_<mode>`` schema is written with: ``config, node, link, geometry, lane, movement,
use_definition, use_group, signal_controller, curb_seg``. Tables that need signal *timing*, travel
demand, or time-of-day data are omitted — OSM doesn't carry them (GMNS's "provide what you have").

    from duckosm.gmns import to_gmns
    to_gmns("data/db/sodermalm.duckdb", "sodermalm_gmns.duckdb")          # standalone GMNS db
    to_gmns("data/db/sodermalm.duckdb", "out.duckdb", to_csv="gmns/")     # + spec CSVs

Needs the DuckDB spatial extension; lane offset geometry needs shapely (geopandas). Lane / signal /
curb detail needs the OSM tags (the ``raw`` schema, present in PBF-mode builds); without them it
degrades to the lane count and skips signal/curb.
"""
import logging
import math
import os

logger = logging.getLogger("duckosm")

# duckOSM mode schema -> GMNS use token, and use -> (persons_per_vehicle, pce)
_MODE_USES = {"driving": "auto", "walking": "walk", "cycling": "bike"}
_USE_DEF = {"auto": (1.0, 1.0), "walk": (1.0, 0.0), "bike": (1.0, 0.2), "bus": (25.0, 2.0)}
_DEFAULT_LANE_W = 3.25          # metres, when width:lanes is absent (used for lane offset spacing)

# GMNS tables that carry a non-spec column for the DuckDB output — dropped for --to-csv fidelity
_CSV_EXCLUDE = {"node": ["geom"], "link": ["geom"], "geometry": ["geom"], "lane": ["geom", "turn"]}


def _mode_schemas(con, src):
    return [r[0] for r in con.execute(
        "SELECT DISTINCT schema_name FROM duckdb_tables() WHERE database_name = ? "
        "AND table_name = 'edges' "
        "AND schema_name NOT IN ('information_schema', 'pg_catalog', 'main', 'raw') "
        "ORDER BY schema_name", [src]).fetchall()]


def _exists(con, src, schema, table):
    return con.execute(
        "SELECT count(*) FROM duckdb_tables() WHERE database_name = ? AND schema_name = ? "
        "AND table_name = ?", [src, schema, table]).fetchone()[0] > 0


def _split(s):
    return s.split("|") if s else []


def _num(s):
    try:
        return float(str(s).strip())
    except (TypeError, ValueError):
        return None


def _offset_wkt(line_wkt, off_m):
    """Offset a lon/lat LINESTRING sideways by ``off_m`` metres (left = +) for lane-level rendering.
    Works in a local metre frame (cos-lat corrected) then back to lon/lat. Returns WKT (or the input
    unchanged if shapely / the offset isn't usable)."""
    if abs(off_m) < 1e-6:
        return line_wkt
    try:
        from shapely import wkt as _w
        from shapely.geometry import LineString, MultiLineString
    except ImportError:
        return line_wkt
    ls = _w.loads(line_wkt)
    if ls.is_empty or ls.length == 0:
        return line_wkt
    x0, y0 = ls.coords[0]
    kx = math.cos(math.radians(y0)) or 1.0
    M = 111320.0
    local = LineString([((x - x0) * kx * M, (y - y0) * M) for x, y in ls.coords])
    try:
        off = local.offset_curve(off_m)
    except Exception:
        return line_wkt
    if off.is_empty:
        return line_wkt
    if isinstance(off, MultiLineString):
        off = max(off.geoms, key=lambda g: g.length)
    back = [(x / (kx * M) + x0, y / M + y0) for x, y in off.coords]
    return LineString(back).wkt if len(back) >= 2 else line_wkt


def to_gmns(source_db, out_path, modes=None, to_csv=None, lane_geometry=True):
    """Extract a built duckOSM db to a standalone GMNS DuckDB.

    Parameters
    ----------
    source_db : path to a built duckOSM ``.duckdb`` (opened read-only).
    out_path : path of the GMNS ``.duckdb`` to create (overwritten if it exists).
    modes : mode schemas to extract (default: every mode present).
    to_csv : if set, also dump spec-standard GMNS CSVs into this directory (per-mode subfolders when
        more than one mode).
    lane_geometry : compute a per-lane offset ``geom`` for lane-level rendering (needs shapely).

    Returns ``{"path", "csv", "modes": {mode: {counts per table}}}``. Every ``link_id`` == ``edge_id``.
    """
    import duckdb

    present_check = duckdb.connect(source_db, read_only=True)
    present_check.close()                                    # fail fast if source is unreadable
    if os.path.exists(out_path):
        os.remove(out_path)
    os.makedirs(os.path.dirname(out_path) or ".", exist_ok=True)

    con = duckdb.connect(out_path)                           # NEW writable target
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute(f"ATTACH '{source_db.replace(chr(39), chr(39) * 2)}' AS s (READ_ONLY)")

    present = _mode_schemas(con, "s")
    chosen = list(modes) if modes else present
    if not chosen:
        con.close()
        raise ValueError("no mode schemas with an 'edges' table in the source db")
    unknown = [m for m in chosen if m not in present]
    if unknown:
        con.close()
        raise ValueError(f"mode(s) {unknown} not found (present: {present or 'none'})")

    has_raw = _exists(con, "s", "raw", "ways")
    if has_raw:
        con.execute("CREATE TEMP TABLE _sig AS "
                    "SELECT DISTINCT osm_id FROM s.raw.nodes WHERE tags['highway'] = 'traffic_signals'")
    else:
        con.execute("CREATE TEMP TABLE _sig(osm_id BIGINT)")
        logger.warning("GMNS: no 'raw' schema — lane/signal/curb detail limited (no OSM tags)")

    name = os.path.splitext(os.path.basename(out_path))[0]
    result = {"path": out_path, "csv": None, "modes": {}}
    for mode in chosen:
        sch = f"gmns_{mode}"
        uses = _MODE_USES.get(mode, mode)
        con.execute(f"CREATE SCHEMA {sch}")
        _build_fixed(con, sch, name, mode, uses)
        _build_node(con, sch, mode)
        _build_link(con, sch, mode, uses, has_raw)
        _build_geometry(con, sch, mode)
        _build_movement(con, sch, mode, uses)
        _build_signal_controller(con, sch)
        _build_lane_curb(con, sch, mode, uses, has_raw, lane_geometry)
        result["modes"][mode] = {
            t: con.execute(f"SELECT count(*) FROM {sch}.{t}").fetchone()[0]
            for t in ("node", "link", "lane", "movement", "signal_controller", "curb_seg")}
        r = result["modes"][mode]
        logger.info(f"GMNS[{mode}]: {r['node']:,} nodes, {r['link']:,} links, {r['lane']:,} lanes, "
                    f"{r['movement']:,} movements, {r['signal_controller']} signals -> {sch}")

    if to_csv:
        _dump_csv(con, chosen, to_csv)
        result["csv"] = to_csv

    con.execute("DETACH s")
    con.close()
    return result


def _build_fixed(con, sch, name, mode, uses):
    """config + use_definition + use_group — the small fixed tables."""
    con.execute(f"""CREATE TABLE {sch}.config AS SELECT * FROM (VALUES
      ('{name}_{mode}', 'meter', 'meter', 'kmh', 'EPSG:4326', 'wkt', '0.97', 'integer')
    ) t(dataset_name, long_length, short_length, speed, crs, geometry_field_format,
        version_number, id_type)""")
    ppv, pce = _USE_DEF.get(uses, (1.0, 1.0))
    con.execute(f"""CREATE TABLE {sch}.use_definition AS SELECT
      '{uses}' AS use, {ppv} AS persons_per_vehicle, {pce} AS pce,
      NULL::VARCHAR AS special_conditions, NULL::VARCHAR AS description""")
    con.execute(f"""CREATE TABLE {sch}.use_group AS SELECT
      '{mode}' AS use_group, '{uses}' AS uses, NULL::VARCHAR AS description""")


def _build_node(con, sch, mode):
    con.execute(f"""CREATE TABLE {sch}.node AS SELECT
      n.node_id, NULL::VARCHAR AS name, ST_X(n.geom) AS x_coord, ST_Y(n.geom) AS y_coord,
      NULL::DOUBLE AS z_coord, NULL::VARCHAR AS node_type,
      CASE WHEN sig.osm_id IS NOT NULL THEN 'signal' END AS ctrl_type,
      NULL::VARCHAR AS zone_id, NULL::BIGINT AS parent_node_id, n.geom
    FROM s.{mode}.nodes n LEFT JOIN _sig sig ON sig.osm_id = n.node_id
    WHERE n.geom IS NOT NULL""")


def _build_link(con, sch, mode, uses, has_raw):
    raw_join = "LEFT JOIN s.raw.ways w ON w.osm_id = e.osm_id" if has_raw else ""
    bike = "w.tags['cycleway']" if has_raw else "NULL::VARCHAR"
    ped = "w.tags['sidewalk']" if has_raw else "NULL::VARCHAR"
    con.execute(f"""CREATE TABLE {sch}.link AS SELECT
      e.edge_id AS link_id, e.name AS name, e.source AS from_node_id, e.target AS to_node_id,
      true AS directed, e.edge_id AS geometry_id, ST_AsText(e.geometry) AS geometry,
      NULL::BIGINT AS parent_link_id, 1 AS dir_flag, e.length_m AS length, NULL::DOUBLE AS grade,
      e.highway AS facility_type, NULL::DOUBLE AS capacity, e.maxspeed_kmh AS free_speed, e.lanes,
      {bike} AS bike_facility, {ped} AS ped_facility, NULL::VARCHAR AS parking,
      '{uses}' AS allowed_uses, NULL::DOUBLE AS toll, NULL::VARCHAR AS jurisdiction,
      NULL::DOUBLE AS row_width, e.geometry AS geom
    FROM s.{mode}.edges e {raw_join}""")


def _build_geometry(con, sch, mode):
    con.execute(f"""CREATE TABLE {sch}.geometry AS SELECT
      edge_id AS geometry_id, ST_AsText(geometry) AS geometry, geometry AS geom
    FROM s.{mode}.edges""")


def _build_movement(con, sch, mode, uses):
    """One row per legal turn from edge_graph; turn `type` from the bearing change at the junction."""
    if not _exists(con, "s", mode, "edge_graph"):
        con.execute(f"""CREATE TABLE {sch}.movement(
          mvmt_id VARCHAR, node_id BIGINT, name VARCHAR, ib_link_id BIGINT, start_ib_lane INT,
          end_ib_lane INT, ob_link_id BIGINT, start_ob_lane INT, end_ob_lane INT, type VARCHAR,
          penalty DOUBLE, capacity DOUBLE, ctrl_type VARCHAR, mvmt_code VARCHAR,
          allowed_uses VARCHAR, geometry VARCHAR)""")
        return
    con.execute(f"""CREATE TABLE {sch}.movement AS
      WITH mv AS (
        SELECT eg.from_edge AS ib, eg.to_edge AS ob, fe.target AS node_id,
               ST_PointN(fe.geometry, ST_NPoints(fe.geometry)::INTEGER)       AS pe,
               ST_PointN(fe.geometry, ST_NPoints(fe.geometry)::INTEGER - 1)   AS pp,
               ST_PointN(te.geometry, 1) AS qs, ST_PointN(te.geometry, 2) AS qn
        FROM s.{mode}.edge_graph eg
        JOIN s.{mode}.edges fe ON fe.edge_id = eg.from_edge
        JOIN s.{mode}.edges te ON te.edge_id = eg.to_edge
        -- drop the immediate reversal (U-turn back onto the same physical segment): an edge_graph
        -- artifact for routing completeness, not a modelled movement. Real intersection U-turns
        -- (a different osm_id) are kept and typed 'uturn'.
        WHERE NOT (te.osm_id = fe.osm_id AND te.source = fe.target AND te.target = fe.source)
      ), a AS (
        SELECT ib, ob, node_id,
          degrees(
            atan2(ST_Y(qn) - ST_Y(qs), (ST_X(qn) - ST_X(qs)) * cos(radians(ST_Y(qs)))) -
            atan2(ST_Y(pe) - ST_Y(pp), (ST_X(pe) - ST_X(pp)) * cos(radians(ST_Y(pe))))
          ) AS raw_ang
        FROM mv
      ), n AS (
        SELECT ib, ob, node_id, ((raw_ang + 180) - floor((raw_ang + 180) / 360) * 360) - 180 AS ang
        FROM a
      )
      SELECT ib::VARCHAR || '-' || ob::VARCHAR AS mvmt_id, node_id, NULL::VARCHAR AS name,
             ib AS ib_link_id, NULL::INT AS start_ib_lane, NULL::INT AS end_ib_lane,
             ob AS ob_link_id, NULL::INT AS start_ob_lane, NULL::INT AS end_ob_lane,
             CASE WHEN abs(ang) >= 150 THEN 'uturn' WHEN abs(ang) < 30 THEN 'thru'
                  WHEN ang >= 30 THEN 'left' ELSE 'right' END AS type,
             NULL::DOUBLE AS penalty, NULL::DOUBLE AS capacity,
             CASE WHEN sig.osm_id IS NOT NULL THEN 'signal' END AS ctrl_type,
             NULL::VARCHAR AS mvmt_code, '{uses}' AS allowed_uses, NULL::VARCHAR AS geometry
      FROM n LEFT JOIN _sig sig ON sig.osm_id = n.node_id""")


def _build_signal_controller(con, sch):
    con.execute(f"""CREATE TABLE {sch}.signal_controller AS SELECT
      'sig_' || node_id::VARCHAR AS controller_id, node_id, 'signal' AS control_type
    FROM {sch}.node WHERE ctrl_type = 'signal'""")


def _build_lane_curb(con, sch, mode, uses, has_raw, lane_geometry):
    """Per-lane rows from OSM lane tags (+ optional offset geometry) and curb_seg from parking tags."""
    import pandas as pd

    raw_join = "LEFT JOIN s.raw.ways w ON w.osm_id = e.osm_id" if has_raw else ""
    tagcol = "w.tags" if has_raw else "NULL::MAP(VARCHAR, VARCHAR)"
    rows = con.execute(f"""SELECT e.edge_id, e.is_reverse, e.lanes, e.length_m, e.source,
      ST_AsText(e.geometry) AS wkt, {tagcol} AS tags
      FROM s.{mode}.edges e {raw_join}""").fetchall()

    def pick(tags, base, is_rev):
        # OSM: unsuffixed *:lanes is the way's forward direction; :backward is the reverse edge
        return tags.get(base + ":backward") if is_rev else (
            tags.get(base + ":forward") or tags.get(base))

    lane_rows, curb_rows = [], []
    for edge_id, is_rev, lanes, length_m, source, wkt, tags in rows:
        tags = tags or {}
        turns = _split(pick(tags, "turn:lanes", is_rev))
        widths = _split(pick(tags, "width:lanes", is_rev))
        bikes = _split(pick(tags, "bicycle:lanes", is_rev))
        psvs = _split(pick(tags, "psv:lanes", is_rev))
        n = len(turns) if turns else int(lanes or 1)
        n = max(n, 1)
        w_each = [(_num(widths[i]) if i < len(widths) else None) or _DEFAULT_LANE_W for i in range(n)]
        half = sum(w_each) / 2.0
        run = 0.0
        for i in range(n):
            u = uses
            if i < len(bikes) and bikes[i] in ("designated", "yes"):
                u = "bike"
            elif i < len(psvs) and psvs[i] in ("designated", "yes"):
                u = "bus"
            width = _num(widths[i]) if i < len(widths) else None
            turn = turns[i] if i < len(turns) and turns[i] not in ("", "none") else None
            off_m = half - (run + w_each[i] / 2.0)          # left (+) → leftmost lane first
            run += w_each[i]
            geom = _offset_wkt(wkt, off_m) if lane_geometry else None
            lane_rows.append((f"{edge_id}_{i + 1}", edge_id, i + 1, u, None, None, width, turn, geom))
        for side in ("left", "right", "both"):
            val = tags.get(f"parking:{side}") or tags.get(f"parking:lane:{side}")
            if val and val not in ("no", "separate", "none"):
                curb_rows.append((f"{edge_id}_{side}", edge_id, source, 0.0, length_m, val, None))

    lane_df = pd.DataFrame(lane_rows, columns=[
        "lane_id", "link_id", "lane_num", "allowed_uses", "r_barrier", "l_barrier",
        "width", "turn", "geom_wkt"])
    con.register("_lane_df", lane_df)
    geom_sel = "ST_GeomFromText(geom_wkt)" if lane_geometry else "NULL::GEOMETRY"
    con.execute(f"""CREATE TABLE {sch}.lane AS SELECT
      lane_id, link_id, lane_num, allowed_uses, r_barrier, l_barrier, width, turn,
      {geom_sel} AS geom FROM _lane_df""")
    con.unregister("_lane_df")

    curb_df = pd.DataFrame(curb_rows, columns=[
        "curb_seg_id", "link_id", "ref_node_id", "start_lr", "end_lr", "regulation", "width"])
    con.register("_curb_df", curb_df)
    con.execute(f"CREATE TABLE {sch}.curb_seg AS SELECT * FROM _curb_df")
    con.unregister("_curb_df")


def _dump_csv(con, modes, to_csv):
    """COPY each GMNS table to a spec-standard CSV (dropping the non-spec geom/turn columns)."""
    tables = ["config", "node", "link", "geometry", "lane", "movement",
              "use_definition", "use_group", "signal_controller", "curb_seg"]
    for mode in modes:
        sch = f"gmns_{mode}"
        d = os.path.join(to_csv, mode) if len(modes) > 1 else to_csv
        os.makedirs(d, exist_ok=True)
        for t in tables:
            excl = _CSV_EXCLUDE.get(t)
            sel = f"* EXCLUDE ({', '.join(excl)})" if excl else "*"
            path = os.path.join(d, f"{t}.csv")
            con.execute(f"COPY (SELECT {sel} FROM {sch}.{t}) TO '{path}' (HEADER, DELIMITER ',')")
