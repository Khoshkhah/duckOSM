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
# saturation capacity default (pce/hr/lane) by facility_type (normalized highway; _link → parent)
_CAPACITY = {"motorway": 2000, "trunk": 1800, "primary": 1600, "secondary": 1400, "tertiary": 1200,
             "unclassified": 1000, "residential": 800, "living_street": 300, "service": 300,
             "road": 800}


def _capacity_case(hw_expr):
    whens = " ".join(f"WHEN '{k}' THEN {v}" for k, v in _CAPACITY.items())
    return f"CASE {hw_expr} {whens} ELSE 800 END"


def _bezier_line_sql(x0, y0, cx0, cy0, cx1, cy1, x1, y1, n=10):
    """A SQL expression: a cubic-Bézier LINESTRING sampled at ``n``+1 points, from endpoints
    (``x0,y0``)→(``x1,y1``) with control points (``cx0,cy0``),(``cx1,cy1``). Used for smooth,
    tangent-respecting turn connectors."""
    ts = ", ".join(f"{i / n:.4f}" for i in range(n + 1))
    bx = f"pow(1-u,3)*{x0} + 3*pow(1-u,2)*u*{cx0} + 3*(1-u)*pow(u,2)*{cx1} + pow(u,3)*{x1}"
    by = f"pow(1-u,3)*{y0} + 3*pow(1-u,2)*u*{cy0} + 3*(1-u)*pow(u,2)*{cy1} + pow(u,3)*{y1}"
    return f"ST_MakeLine(list_transform([{ts}], u -> ST_Point({bx}, {by})))"

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


def _placement(value, w_each):
    """OSM ``placement`` (``left_of:N`` / ``middle_of:N`` / ``right_of:N``) -> where the way's line
    lies across its lanes, in metres from the left edge of lane 1 (lanes left to right in the
    direction of travel). None for ``transition``, a missing tag or a lane number the edge doesn't
    have: then the default placement stands (docs/design/gmns_lane_movements.md, step 6)."""
    try:
        kind, n = str(value).split(":")
        n = int(n)
    except ValueError:
        return None
    if not 1 <= n <= len(w_each):
        return None
    left = sum(w_each[:n - 1])
    return {"left_of": left, "middle_of": left + w_each[n - 1] / 2, "right_of": left + w_each[n - 1]}.get(kind)


def _paired_gaps(edges, side_sign, step_m=2.0):
    """One-way edges placed as one side of a two-way road (docs/design/gmns_paired_carriageways.md).

    ``edges``: ``(edge_id, wkt, half_width_m)`` of the one-way edges. Returns ``{edge_id: d}``, the
    median gap in metres between an edge's centre line and its partner(s): one-way edges running the
    opposite way on its inner side (left for right-hand traffic) closer than ``half_A + half_B``,
    alongside it for at least half its length. Sampled every ``step_m`` metres."""
    import statistics

    from shapely import STRtree
    from shapely import wkt as _w
    from shapely.geometry import LineString

    geoms = [_w.loads(w) for _, w, _ in edges]
    if not geoms:
        return {}
    x0, y0 = geoms[0].coords[0]                          # one local metre frame for the whole area
    kx, M = (math.cos(math.radians(y0)) or 1.0), 111320.0
    lines = [LineString([((x - x0) * kx * M, (y - y0) * M) for x, y in g.coords]) for g in geoms]
    half = [h for _, _, h in edges]
    tree = STRtree(lines)
    reach = max(half)

    def tangent(ln, s):
        a, b = ln.interpolate(max(s - 0.5, 0)), ln.interpolate(min(s + 0.5, ln.length))
        dx, dy = b.x - a.x, b.y - a.y
        n = math.hypot(dx, dy) or 1.0
        return dx / n, dy / n

    gaps = {}
    for i, A in enumerate(lines):
        if A.length < step_m:
            continue
        cand = [j for j in tree.query(A.buffer(half[i] + reach)) if j != i]
        if not cand:
            continue
        found, samples = [], 0
        for k in range(int(A.length // step_m) + 1):
            s = min(k * step_m, A.length)
            p, (tx, ty) = A.interpolate(s), tangent(A, s)
            samples += 1
            best = None
            for j in cand:
                B = lines[j]
                sb = B.project(p)
                q = B.interpolate(sb)
                d = p.distance(q)
                if d >= half[i] + half[j] or (best is not None and d >= best):
                    continue
                bx, by = tangent(B, sb)
                cross = tx * (q.y - p.y) - ty * (q.x - p.x)  # > 0: q is left of A
                if tx * bx + ty * by < -0.866 and cross * side_sign < 0:
                    best = d
            if best is not None:
                found.append(best)
        if found and len(found) >= samples / 2:
            gaps[edges[i][0]] = statistics.median(found)
    return gaps


def to_gmns(source_db, out_path, modes=None, to_csv=None, lane_geometry=True, combined=False,
            drive_side="right", pair_carriageways=True):
    """Extract a built duckOSM db to a standalone GMNS DuckDB.

    Parameters
    ----------
    source_db : path to a built duckOSM ``.duckdb`` (opened read-only).
    out_path : path of the GMNS ``.duckdb`` to create (overwritten if it exists).
    modes : mode schemas to extract (default: every mode present).
    to_csv : if set, also dump spec-standard GMNS CSVs into this directory (per-mode subfolders when
        more than one mode).
    lane_geometry : compute a per-lane offset ``geom`` for lane-level rendering (needs shapely).
    pair_carriageways : place a one-way edge with an opposite one-way partner close on its inner side
        as one side of a two-way road, from the line midway between them (default; False = centred
        on its own way, as before). docs/design/gmns_paired_carriageways.md
    combined : also write a single **mode-tagged** ``gmns_all`` network (node + link), the per-mode
        links merged on ``link_id`` (= ``edge_id``) with ``allowed_uses`` unioned across the modes that
        contain each edge. Needs ≥2 modes.

    Returns ``{"path", "csv", "combined", "modes": {mode: {counts per table}}}``. Every
    ``link_id`` == ``edge_id``.
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
        _build_lane_curb(con, sch, mode, uses, has_raw, lane_geometry, drive_side,
                         pair_carriageways)  # before movement
        _build_movement(con, sch, mode, uses, drive_side)
        if lane_geometry:
            _build_lane_connectors(con, sch)            # lane ends + connectors, after the lane ranges
        _build_signal_controller(con, sch)
        result["modes"][mode] = {
            t: con.execute(f"SELECT count(*) FROM {sch}.{t}").fetchone()[0]
            for t in ("node", "link", "lane", "movement", "signal_controller", "curb_seg")}
        r = result["modes"][mode]
        logger.info(f"GMNS[{mode}]: {r['node']:,} nodes, {r['link']:,} links, {r['lane']:,} lanes, "
                    f"{r['movement']:,} movements, {r['signal_controller']} signals -> {sch}")

    result["combined"] = None
    if combined and len(chosen) > 1:
        c = _build_combined(con, chosen, name)
        result["combined"] = c
        logger.info(f"GMNS[combined]: {c['node']:,} nodes, {c['link']:,} links "
                    f"(mode-tagged allowed_uses) -> gmns_all")

    if to_csv:
        _dump_csv(con, chosen, to_csv)
        if result["combined"]:
            _dump_csv(con, ["all"], os.path.join(to_csv, "combined"), schema_prefix="gmns_")
        result["csv"] = to_csv

    con.execute("DETACH s")
    con.close()
    return result


def _build_combined(con, modes, name):
    """A single mode-tagged ``gmns_all`` network: per-mode node/link merged on the shared id
    (``edge_id`` collides across modes → the same physical edge becomes one row), with
    ``allowed_uses`` unioned across the modes that contain it. Lane/movement stay per-mode."""
    con.execute("CREATE SCHEMA gmns_all")
    con.execute(f"""CREATE TABLE gmns_all.config AS SELECT * FROM (VALUES
      ('{name}_all', 'meter', 'meter', 'kmh', 'EPSG:4326', 'wkt', '0.97', 'integer')
    ) t(dataset_name, long_length, short_length, speed, crs, geometry_field_format,
        version_number, id_type)""")
    con.execute("CREATE TABLE gmns_all.use_definition AS "
                + " UNION ".join(f"SELECT * FROM gmns_{m}.use_definition" for m in modes))
    con.execute("CREATE TABLE gmns_all.use_group AS "
                + " UNION ".join(f"SELECT * FROM gmns_{m}.use_group" for m in modes))
    con.execute(f"""CREATE TABLE gmns_all.node AS
      WITH u AS ({" UNION ALL ".join(f"SELECT * FROM gmns_{m}.node" for m in modes)})
      SELECT node_id, any_value(name) AS name, any_value(x_coord) AS x_coord,
             any_value(y_coord) AS y_coord, any_value(z_coord) AS z_coord,
             any_value(node_type) AS node_type, max(ctrl_type) AS ctrl_type,
             any_value(zone_id) AS zone_id, any_value(parent_node_id) AS parent_node_id,
             any_value(geom) AS geom
      FROM u GROUP BY node_id""")
    con.execute(f"""CREATE TABLE gmns_all.link AS
      WITH u AS ({" UNION ALL ".join(f"SELECT * FROM gmns_{m}.link" for m in modes)})
      SELECT link_id, any_value(name) AS name, any_value(from_node_id) AS from_node_id,
             any_value(to_node_id) AS to_node_id, any_value(directed) AS directed,
             any_value(geometry_id) AS geometry_id, any_value(geometry) AS geometry,
             any_value(parent_link_id) AS parent_link_id, any_value(dir_flag) AS dir_flag,
             any_value(length) AS length, any_value(grade) AS grade,
             any_value(facility_type) AS facility_type, any_value(capacity) AS capacity,
             any_value(free_speed) AS free_speed, any_value(lanes) AS lanes,
             any_value(bike_facility) AS bike_facility, any_value(ped_facility) AS ped_facility,
             any_value(parking) AS parking,
             string_agg(DISTINCT allowed_uses, ',' ORDER BY allowed_uses) AS allowed_uses,
             any_value(toll) AS toll, any_value(jurisdiction) AS jurisdiction,
             any_value(row_width) AS row_width, any_value(geom) AS geom
      FROM u GROUP BY link_id""")
    return {t: con.execute(f"SELECT count(*) FROM gmns_all.{t}").fetchone()[0]
            for t in ("node", "link")}


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
      e.highway AS facility_type,
      {_capacity_case("regexp_replace(split_part(e.highway, ';', 1), '_link$', '')")}::DOUBLE AS capacity,
      e.maxspeed_kmh AS free_speed, e.lanes,
      {bike} AS bike_facility, {ped} AS ped_facility, NULL::VARCHAR AS parking,
      '{uses}' AS allowed_uses, NULL::DOUBLE AS toll, NULL::VARCHAR AS jurisdiction,
      NULL::DOUBLE AS row_width, e.geometry AS geom
    FROM s.{mode}.edges e {raw_join}""")


def _build_geometry(con, sch, mode):
    con.execute(f"""CREATE TABLE {sch}.geometry AS SELECT
      edge_id AS geometry_id, ST_AsText(geometry) AS geometry, geometry AS geom
    FROM s.{mode}.edges""")


def _build_movement(con, sch, mode, uses, drive_side="right"):
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
               fe.geometry AS ibg, te.geometry AS obg,
               ST_PointN(fe.geometry, ST_NPoints(fe.geometry)::INTEGER)       AS pe,
               ST_PointN(fe.geometry, ST_NPoints(fe.geometry)::INTEGER - 1)   AS pp,
               ST_PointN(te.geometry, 1) AS qs, ST_PointN(te.geometry, 2) AS qn
        FROM s.{mode}.edge_graph eg
        JOIN s.{mode}.edges fe ON fe.edge_id = eg.from_edge
        JOIN s.{mode}.edges te ON te.edge_id = eg.to_edge
        -- drop the immediate reversal (U-turn back onto the same physical segment): an edge_graph
        -- artifact for routing completeness, not a modelled movement -- except where turning round
        -- is the only way on (a dead end) or the only way into the reverse edge (every other road
        -- at the node leaves it, Monaco Avenue de l'Annonciade): lane routing would be stuck, the
        -- lane unreachable. Real intersection U-turns (a different osm_id) are kept too; all 'uturn'.
        WHERE NOT (te.osm_id = fe.osm_id AND te.source = fe.target AND te.target = fe.source
                   AND EXISTS (SELECT 1 FROM s.{mode}.edge_graph o
                               WHERE o.from_edge = eg.from_edge AND o.to_edge <> eg.to_edge)
                   AND EXISTS (SELECT 1 FROM s.{mode}.edge_graph i
                               WHERE i.to_edge = eg.to_edge AND i.from_edge <> eg.from_edge))
      ), b AS (
        SELECT ib, ob, node_id, ibg, obg,
          atan2(ST_Y(pe) - ST_Y(pp), (ST_X(pe) - ST_X(pp)) * cos(radians(ST_Y(pe)))) AS in_b,
          atan2(ST_Y(qn) - ST_Y(qs), (ST_X(qn) - ST_X(qs)) * cos(radians(ST_Y(qs)))) AS out_b
        FROM mv
      ), t AS (
        SELECT ib, ob, node_id, ibg, obg, ang,
          CASE WHEN abs(ang) >= 150 THEN 'uturn' WHEN abs(ang) < 30 THEN 'thru'
               WHEN ang >= 30 THEN 'left' ELSE 'right' END AS type,
          -- inbound compass heading (0=N, clockwise) = (90 - math-bearing) mod 360
          ((90 - degrees(in_b)) - floor((90 - degrees(in_b)) / 360) * 360) AS hdg
        FROM (SELECT *, ((degrees(out_b - in_b) + 180) - floor((degrees(out_b - in_b) + 180) / 360) * 360) - 180 AS ang FROM b)
      ), pts AS (                        -- turn-connector endpoints + tangent anchors, hugging the junction
        SELECT t.*,
          ST_X(ST_LineInterpolatePoint(ibg, 0.94)) AS x0, ST_Y(ST_LineInterpolatePoint(ibg, 0.94)) AS y0,
          ST_X(ST_LineInterpolatePoint(ibg, 0.78)) AS xb, ST_Y(ST_LineInterpolatePoint(ibg, 0.78)) AS yb,
          ST_X(ST_LineInterpolatePoint(obg, 0.06)) AS x1, ST_Y(ST_LineInterpolatePoint(obg, 0.06)) AS y1,
          ST_X(ST_LineInterpolatePoint(obg, 0.22)) AS xa, ST_Y(ST_LineInterpolatePoint(obg, 0.22)) AS ya
        FROM t
      ), cp AS (                         -- cubic-Bézier control points (tangent-respecting)
        SELECT pts.*,
          x0 + (x0 - xb) / GREATEST(sqrt(pow(x0-xb,2)+pow(y0-yb,2)), 1e-9)
               * (0.3 * sqrt(pow(x1-x0,2)+pow(y1-y0,2))) AS cx0,
          y0 + (y0 - yb) / GREATEST(sqrt(pow(x0-xb,2)+pow(y0-yb,2)), 1e-9)
               * (0.3 * sqrt(pow(x1-x0,2)+pow(y1-y0,2))) AS cy0,
          x1 - (xa - x1) / GREATEST(sqrt(pow(xa-x1,2)+pow(ya-y1,2)), 1e-9)
               * (0.3 * sqrt(pow(x1-x0,2)+pow(y1-y0,2))) AS cx1,
          y1 - (ya - y1) / GREATEST(sqrt(pow(xa-x1,2)+pow(ya-y1,2)), 1e-9)
               * (0.3 * sqrt(pow(x1-x0,2)+pow(y1-y0,2))) AS cy1
        FROM pts
      )
      SELECT t.ib::VARCHAR || '-' || t.ob::VARCHAR AS mvmt_id, t.node_id, NULL::VARCHAR AS name,
             t.ib AS ib_link_id, NULL::INT AS start_ib_lane, NULL::INT AS end_ib_lane,
             t.ob AS ob_link_id, NULL::INT AS start_ob_lane, NULL::INT AS end_ob_lane, t.type,
             NULL::DOUBLE AS penalty, NULL::DOUBLE AS capacity,
             CASE WHEN sig.osm_id IS NOT NULL THEN 'signal' END AS ctrl_type,
             (CASE WHEN t.hdg < 45 OR t.hdg >= 315 THEN 'NB' WHEN t.hdg < 135 THEN 'EB'
                   WHEN t.hdg < 225 THEN 'SB' ELSE 'WB' END)
             || (CASE t.type WHEN 'left' THEN 'L' WHEN 'right' THEN 'R'
                             WHEN 'uturn' THEN 'U' ELSE 'T' END) AS mvmt_code,
             '{uses}' AS allowed_uses,
             ST_AsText({_bezier_line_sql('t.x0', 't.y0', 't.cx0', 't.cy0',
                                         't.cx1', 't.cy1', 't.x1', 't.y1')}) AS geometry,
             t.ang AS _ang
      FROM cp t
      LEFT JOIN _sig sig ON sig.osm_id = t.node_id""")
    _assign_lanes(con, sch, mode, drive_side)


# turn:lanes part -> the movement types a lane feeds (docs/design/gmns_lane_movements.md, step 1); a
# slight turn under 30 degrees is typed thru by its angle, so slight_* feeds thru too
_TURN_KINDS = {"through": {"thru"}, "left": {"left"}, "sharp_left": {"left"}, "slight_left": {"left", "thru"},
               "right": {"right"}, "sharp_right": {"right"}, "slight_right": {"right", "thru"},
               "reverse": {"uturn"}, "merge_to_left": {"thru"}, "merge_to_right": {"thru"},
               "none": {"thru"}, "": {"thru"}}


def _turn_kinds(turn):
    """One lane's ``turn:lanes`` value (``through;right``) -> the movement types it feeds."""
    return set().union(*(_TURN_KINDS.get(p.strip(), set()) for p in str(turn).split(";")))


def _default_lanes(n, obs):
    """osm2gmns 0.7.6's lane rule (movement/autoconintd.py), for an inbound link with ``n`` lanes and
    its outbound links' lane counts ``obs``, sorted left to right. Per outbound link a pair of 0-based
    ranges ``((ib0, ib1), (ob0, ob1))`` of equal length, read in order; None where it gets no lane.
    Separate lanes per turn: the leftmost link the leftmost lane, the rightmost the rightmost, the
    ones between share the rest (docs/design/gmns_lane_movements.md, steps 2-3)."""
    k, out = len(obs), [None] * len(obs)
    if n == 1:
        out[0] = ((0, 0), (0, 0))
        for j in range(1, k):
            out[j] = ((0, 0), (obs[j] - 1, obs[j] - 1))
        return out
    if k == 1:
        c = min(n, obs[0])
        return [((0, c - 1), (0, c - 1))]
    if k == 2:
        c = min(n - 1, obs[0])
        return [((0, c - 1), (0, c - 1)), ((n - 1, n - 1), (obs[1] - 1, obs[1] - 1))]
    out[0], mids = ((0, 0), (0, 0)), list(range(1, k - 1))
    if n - 2 >= len(mids):                       # enough middle lanes: deal them out in turn
        left, room, got = n - 2, [obs[j] for j in mids], [0] * len(mids)
        while left > 0 and sum(room) > 0:
            for x in range(len(mids)):
                if room[x] == 0:
                    continue
                if left == 0:
                    break
                room[x], got[x], left = room[x] - 1, got[x] + 1, left - 1
        start = 1
        for x, j in enumerate(mids):
            if got[x]:
                out[j] = ((start, start + got[x] - 1), (obs[j] - got[x], obs[j] - 1))
            start += got[x]
    elif n < len(mids):                          # fewer lanes than middle links: the last lane shared
        for x, j in enumerate(mids):
            lane = min(x, n - 1)
            out[j] = ((lane, lane), (obs[j] - 1, obs[j] - 1))
    else:                                        # one lane per middle link
        start = 1 if n - 1 == len(mids) else 0
        for x, j in enumerate(mids):
            out[j] = ((start + x, start + x), (obs[j] - 1, obs[j] - 1))
    out[-1] = ((n - 1, n - 1), (obs[-1] - 1, obs[-1] - 1))
    return out


def _turn_side(ms):
    """The ``turn:lanes`` arrow each exit takes, by its place among the exits (sorted left to right),
    not by the angle type: the straightest exit within 45 degrees is ``thru``, exits left of it
    ``left``, right of it ``right``; a U-turn ``uturn``. So a slight right fork typed ``thru`` by its
    angle still takes the right-turn lanes (Monaco, Boulevard Charles III)."""
    real = [k for k, m in enumerate(ms) if m[2] != "uturn"]
    s = min(real, key=lambda k: abs(ms[k][3]), default=None)
    s = s if s is not None and abs(ms[s][3]) < 45 else None
    return ["uturn" if m[2] == "uturn" else "thru" if k == s
            else ("left" if (k < s if s is not None else m[3] > 0) else "right") for k, m in enumerate(ms)]


def _merge_lanes(ns, m):
    """osm2gmns 0.7.6's merge rule (movement/autoconm.py): inbound links with ``ns`` lanes, sorted left
    to right, joining one outbound link of ``m`` lanes. The leftmost link's rightmost lanes go into
    the outbound's leftmost lanes, every other link's leftmost lanes into its rightmost lanes. Per
    inbound link a pair of 0-based ranges ``((ib0, ib1), (ob0, ob1))`` of equal length."""
    out = []
    for k, n in enumerate(ns):
        c = min(m, n)
        out.append(((n - c, n - 1), (0, c - 1)) if k == 0 else ((0, c - 1), (m - c, m - 1)))
    return out


def _assign_lanes(con, sch, mode, drive_side="right"):
    """Fill each movement's inbound / outbound lane ranges (docs/design/gmns_lane_movements.md):

    - ``turn:lanes`` only on the last piece of its OSM way, at the junction where the way ends (the
      arrows apply "to the junction", OSM wiki Key:turn): each exit takes the lanes whose arrow
      matches its place among the exits (``_turn_side``);
    - along a way (a movement into the next piece of the same way, same direction) every lane
      continues lane by lane, so marked lanes run on to their junction;
    - into a merge (a node with one outbound link), osm2gmns's merge rule (``_merge_lanes``);
    - else osm2gmns's junction rule (``_default_lanes``).

    Ranges have equal length and pair in order: the k-th inbound lane into the k-th outbound lane.
    Then the GMNS types ``diverge`` / ``merge`` (``_fork_types``). Drops the helper column ``_ang``."""
    from collections import defaultdict

    import pandas as pd

    nl = dict(con.execute(f"SELECT link_id, max(lane_num) FROM {sch}.lane GROUP BY 1").fetchall())
    kinds = defaultdict(dict)        # link -> {lane_num: types}, for links with turn:lanes; a lane left
    for lk, num, turn in con.execute(  # empty there ('|' or 'none', stored NULL) has no arrow: straight on
            f"SELECT link_id, lane_num, turn FROM {sch}.lane WHERE link_id IN "
            f"(SELECT link_id FROM {sch}.lane WHERE turn IS NOT NULL)").fetchall():
        kinds[lk][num] = _turn_kinds(turn) if turn is not None else {"thru"}
    way = {}                         # edge -> (OSM way, direction), to tell a way's pieces apart
    if _exists(con, "s", mode, "edges"):
        rev = "is_reverse" if "is_reverse" in {c for (c,) in con.execute(
            "SELECT column_name FROM duckdb_columns() WHERE database_name = 's' AND schema_name = ? "
            "AND table_name = 'edges'", [mode]).fetchall()} else "false"
        way = {e: (o, r) for e, o, r in con.execute(f"SELECT edge_id, osm_id, {rev} FROM s.{mode}.edges").fetchall()}
    by_ib = defaultdict(list)
    for mid, ib, ob, typ, ang in con.execute(f"SELECT mvmt_id, ib_link_id, ob_link_id, type, _ang FROM {sch}.movement").fetchall():
        by_ib[ib].append((mid, ob, typ, ang or 0.0))
    along = lambda ib, ob: ib in way and way.get(ob) == way[ib]      # the next piece of the same way
    # merges as osm2gmns sees them: a node with one outbound link; its inbound links (bar the outbound
    # link's own reverse), sorted left to right by the turn into it
    ends = dict(con.execute(f"SELECT link_id, to_node_id FROM {sch}.link").fetchall())
    starts = dict(con.execute(f"SELECT link_id, from_node_id FROM {sch}.link").fetchall())
    outs = defaultdict(list)
    for lk, nd in starts.items():
        outs[nd].append(lk)
    merge = {}                                   # movement id -> its ranges by the merge rule
    into = defaultdict(list)
    for ib, ms in by_ib.items():
        for mid, ob, typ, ang in ms:
            if (typ != "uturn" and len(outs.get(starts.get(ob), [])) == 1
                    and starts.get(ib) != ends.get(ob)):            # not the outbound link's own reverse
                into[ob].append((ang, mid, ib))
    for ob, ins in into.items():
        if len(ins) >= 2:
            ins.sort(key=lambda x: x[0], reverse=True)
            for (_, mid, ib), rng in zip(ins, _merge_lanes([nl.get(x[2], 1) for x in ins], nl.get(ob, 1))):
                merge[mid] = rng
    uturn_side = 180.0 if drive_side == "right" else -180.0    # a U-turn is the leftmost (right-hand traffic)
    rows = []
    for ib, ms in by_ib.items():
        n = nl.get(ib, 1)
        ms.sort(key=lambda m: uturn_side if m[2] == "uturn" else m[3], reverse=True)   # left to right
        obs = [nl.get(m[1], 1) for m in ms]
        ends = not any(along(ib, m[1]) for m in ms)      # the way ends here: its arrows apply
        for (mid, ob, typ, _), mo, rng, side in zip(ms, obs, _default_lanes(n, obs), _turn_side(ms)):
            tagged = sorted(num for num, ks in kinds.get(ib, {}).items() if side in ks) if ends else []
            if along(ib, ob):                    # along the way: every lane on, lane by lane
                c = min(n, mo)
                rng = ((0, c - 1), (0, c - 1))
            elif tagged:                         # turn:lanes: its lanes, into as many lanes as fit
                a, c = tagged[0] - 1, min(tagged[-1] - tagged[0] + 1, mo)
                rng = ((a, a + c - 1), (0, c - 1) if side in ("left", "uturn") else (mo - c, mo - 1))
            elif mid in merge:                   # joining at a merge: stacked side by side
                rng = merge[mid]
            if rng is None:                      # osm2gmns gives it no lane: the rightmost pair
                rng = ((n - 1, n - 1), (mo - 1, mo - 1))
            (i0, i1), (o0, o1) = rng
            rows.append((mid, i0 + 1, i1 + 1, o0 + 1, o1 + 1))
    if rows:
        _lanes = pd.DataFrame(rows, columns=["mvmt_id", "si", "ei", "so", "eo"])  # noqa: F841 (read by SQL)
        con.execute(f"""UPDATE {sch}.movement m SET start_ib_lane = l.si, end_ib_lane = l.ei,
                          start_ob_lane = l.so, end_ob_lane = l.eo FROM _lanes l WHERE l.mvmt_id = m.mvmt_id""")
    _fork_types(con, sch)
    con.execute(f"ALTER TABLE {sch}.movement DROP COLUMN _ang")


def _fork_types(con, sch, max_ang=45.0):
    """The GMNS movement types ``diverge`` and ``merge`` (docs/design/gmns_lane_movements.md):
    ``diverge`` at a fork, a node one link arrives at, its 2+ ways on (U-turns aside) all within
    ``max_ang`` degrees of straight on; ``merge`` into a node one link leaves, its 2+ inbound links
    all joining within ``max_ang``. The angle bound keeps an ordinary junction (a one-way street
    meeting a cross street) a junction. ``mvmt_code`` keeps the angle's letter (GMNS allows R/L/T)."""
    con.execute(f"""UPDATE {sch}.movement m SET type = 'diverge' WHERE type <> 'uturn' AND m.node_id IN (
        SELECT f.node_id FROM {sch}.movement f
        WHERE (SELECT count(*) FROM {sch}.link k WHERE k.to_node_id = f.node_id) = 1 AND f.type <> 'uturn'
        GROUP BY f.node_id HAVING count(*) >= 2 AND max(abs(f._ang)) < {max_ang})""")
    con.execute(f"""UPDATE {sch}.movement m SET type = 'merge' WHERE type <> 'uturn' AND m.ob_link_id IN (
        SELECT f.ob_link_id FROM {sch}.movement f
        WHERE (SELECT count(*) FROM {sch}.link k WHERE k.from_node_id = f.node_id) = 1 AND f.type <> 'uturn'
        GROUP BY f.ob_link_id HAVING count(*) >= 2 AND max(abs(f._ang)) < {max_ang})""")


def _build_lane_connectors(con, sch, gap_ok=0.25, max_trim=0.4, junction_pad=0.5):
    """Lanes that connect (docs/design/gmns_lane_connectors.md): shorten each lane where it ends
    inside a junction (the other links' lanes, but its own link's reverse) or doesn't meet the lane
    it leads into, then join every lane pair of a movement with a cubic Bézier along both lanes, in
    ``lane_connector``. Works in one local metre frame for the area."""
    import shapely
    from shapely import wkt as _w
    from shapely.geometry import LineString, Point
    from shapely.ops import substring

    lanes = con.execute(f"SELECT lane_id, link_id, lane_num, COALESCE(width, {_DEFAULT_LANE_W}), ST_AsText(geom) "
                        f"FROM {sch}.lane WHERE geom IS NOT NULL").fetchall()
    con.execute(f"""CREATE TABLE {sch}.lane_connector(connector_id VARCHAR, mvmt_id VARCHAR, from_lane_id VARCHAR,
                      to_lane_id VARCHAR, width DOUBLE, geom GEOMETRY)""")
    if not lanes:
        return
    x0, y0 = _w.loads(lanes[0][4]).coords[0]
    kx, M = (math.cos(math.radians(y0)) or 1.0), 111320.0
    to_m = lambda g: LineString([((x - x0) * kx * M, (y - y0) * M) for x, y in g.coords])
    geom = {lid: to_m(_w.loads(w)) for lid, _, _, _, w in lanes}
    link_of = {lid: lk for lid, lk, _, _, _ in lanes}
    width = {lid: w for lid, _, _, w, _ in lanes}
    by_link = {}
    for lid, lk, num, _, _ in lanes:
        by_link.setdefault(lk, {})[num] = lid
    ends = {lk: (a, b) for lk, a, b in con.execute(f"SELECT link_id, from_node_id, to_node_id FROM {sch}.link").fetchall()}
    nb = {}
    for a, b in ends.values():
        nb.setdefault(a, set()).add(b)
        nb.setdefault(b, set()).add(a)
    at = {}
    for lk, (a, b) in ends.items():
        at.setdefault(a, set()).add(lk)
        at.setdefault(b, set()).add(lk)
    surface = {lk: shapely.union_all([geom[l].buffer(width[l] / 2, cap_style="flat") for l in ls.values() if l in geom])
               for lk, ls in by_link.items()}
    pairs = []                                   # (mvmt_id, from lane, to lane): the k-th into the k-th
    for mid, ib, ob, si, ei, so in con.execute(
            f"SELECT mvmt_id, ib_link_id, ob_link_id, start_ib_lane, end_ib_lane, start_ob_lane FROM {sch}.movement "
            f"WHERE start_ib_lane IS NOT NULL AND start_ob_lane IS NOT NULL").fetchall():
        for k in range(ei - si + 1):
            a, b = by_link.get(ib, {}).get(si + k), by_link.get(ob, {}).get(so + k)
            if a in geom and b in geom:
                pairs.append((mid, a, b))

    def inside_len(line, node, link, at_end):
        """How far from its end (at_end) or start the lane lies inside the other links' lanes at a junction."""
        if len(nb.get(node, ())) < 3:
            return 0.0
        a, b = ends[link]
        other = [surface[k] for k in at.get(node, ()) if k != link and k in surface and ends[k] != (b, a)]
        if not other:
            return 0.0
        u, n, step = shapely.union_all(other), line.length, 0.5
        s = 0.0
        while s < n * max_trim and u.contains(line.interpolate(n - s if at_end else s)):
            s += step
        return s + junction_pad if s else 0.0

    trim = {lid: [0.0, 0.0] for lid in geom}     # [at the start, at the end] in metres
    for lid, line in geom.items():
        a, b = ends[link_of[lid]]
        trim[lid] = [inside_len(line, a, link_of[lid], False), inside_len(line, b, link_of[lid], True)]
    for _, a, b in pairs:                        # room for an S-curve where the lanes don't meet
        gap = Point(geom[a].coords[-1]).distance(Point(geom[b].coords[0]))
        if gap > gap_ok:
            need = max(3.0, 2.5 * gap) / 2
            trim[a][1], trim[b][0] = max(trim[a][1], need), max(trim[b][0], need)
    cut = {}
    for lid, line in geom.items():
        n = line.length
        s0, s1 = (min(t, n * max_trim) for t in trim[lid])
        cut[lid] = substring(line, s0, n - s1) if s0 or s1 else line

    def tangent(line, at_end):
        n = line.length
        p, q = (line.interpolate(max(n - 1.0, 0)), line.interpolate(n)) if at_end else (line.interpolate(0), line.interpolate(min(1.0, n)))
        dx, dy = q.x - p.x, q.y - p.y
        d = math.hypot(dx, dy) or 1.0
        return dx / d, dy / d

    to_ll = lambda pts: "LINESTRING(" + ", ".join(f"{x / (kx * M) + x0:.8f} {y / M + y0:.8f}" for x, y in pts) + ")"
    rows = []
    for mid, a, b in pairs:
        p0, p3 = cut[a].coords[-1], cut[b].coords[0]
        chord = math.dist(p0, p3)
        if chord < 0.3:
            continue
        (ax, ay), (bx, by) = tangent(cut[a], True), tangent(cut[b], False)
        p1, p2 = (p0[0] + ax * 0.4 * chord, p0[1] + ay * 0.4 * chord), (p3[0] - bx * 0.4 * chord, p3[1] - by * 0.4 * chord)
        pts = [tuple((1 - t) ** 3 * u + 3 * (1 - t) ** 2 * t * v + 3 * (1 - t) * t ** 2 * w + t ** 3 * z
                     for u, v, w, z in zip(p0, p1, p2, p3)) for t in (i / 12 for i in range(13))]
        rows.append((f"{a}>{b}", mid, a, b, min(width[a], width[b]), to_ll(pts)))
    import pandas as pd
    _cut = pd.DataFrame([(lid, to_ll(line.coords)) for lid, line in cut.items() if line is not geom[lid]],  # noqa: F841
                        columns=["lane_id", "wkt"])
    if len(_cut):
        con.execute(f"UPDATE {sch}.lane l SET geom = ST_GeomFromText(c.wkt) FROM _cut c WHERE c.lane_id = l.lane_id")
    if rows:
        _con = pd.DataFrame(rows, columns=["connector_id", "mvmt_id", "from_lane_id", "to_lane_id", "width", "wkt"])  # noqa: F841
        con.execute(f"INSERT INTO {sch}.lane_connector SELECT connector_id, mvmt_id, from_lane_id, to_lane_id, width, "
                    f"ST_GeomFromText(wkt) FROM _con")


def _build_signal_controller(con, sch):
    con.execute(f"""CREATE TABLE {sch}.signal_controller AS SELECT
      'sig_' || node_id::VARCHAR AS controller_id, node_id, 'signal' AS control_type
    FROM {sch}.node WHERE ctrl_type = 'signal'""")


def _build_lane_curb(con, sch, mode, uses, has_raw, lane_geometry, drive_side="right",
                     pair_carriageways=True):
    """Per-lane rows from OSM lane tags (+ optional offset geometry) and curb_seg from parking tags.

    Lane offset is **drive-side aware**: a one-way road's lanes are centered on its carriageway, but a
    two-way road's lanes are shifted to the direction's travel side (right for ``drive_side='right'``),
    so the forward and reverse edges separate onto opposite physical sides instead of overlapping.
    A one-way edge with an opposite one-way partner closer than their lanes need (``_paired_gaps``)
    is placed like a two-way road's side, from the line midway between the two."""
    import pandas as pd

    raw_join = "LEFT JOIN s.raw.ways w ON w.osm_id = e.osm_id" if has_raw else ""
    tagcol = "w.tags" if has_raw else "NULL::MAP(VARCHAR, VARCHAR)"
    has_oneway = con.execute(
        "SELECT count(*) FROM duckdb_columns() WHERE database_name = 's' AND schema_name = ? "
        "AND table_name = 'edges' AND column_name = 'oneway'", [mode]).fetchone()[0] > 0
    oneway_sel = "e.oneway" if has_oneway else "false"      # older/synthetic edges: treat as two-way
    rows = con.execute(f"""SELECT e.edge_id, e.is_reverse, e.lanes, e.length_m, e.source,
      {oneway_sel} AS oneway, ST_AsText(e.geometry) AS wkt, {tagcol} AS tags
      FROM s.{mode}.edges e {raw_join}""").fetchall()
    side_sign = -1.0 if drive_side == "right" else 1.0  # offset_curve(+) is left; right-hand → negative

    def pick(tags, base, is_rev):
        # OSM: unsuffixed *:lanes is the way's forward direction; :backward is the reverse edge
        return tags.get(base + ":backward") if is_rev else (
            tags.get(base + ":forward") or tags.get(base))

    def lane_widths(tags, lanes, is_rev):
        turns = _split(pick(tags, "turn:lanes", is_rev))
        widths = _split(pick(tags, "width:lanes", is_rev))
        n = max(len(turns) if turns else int(lanes or 1), 1)
        return turns, widths, [(_num(widths[i]) if i < len(widths) else None) or _DEFAULT_LANE_W
                               for i in range(n)]

    gaps = {}
    if lane_geometry and pair_carriageways:
        gaps = _paired_gaps([(r[0], r[6], sum(lane_widths(r[7] or {}, r[2], r[1])[2]) / 2.0)
                             for r in rows if r[5] and r[6]], side_sign)
        if gaps:
            logger.info(f"GMNS[{mode}]: {len(gaps):,} one-way edges placed with their opposite carriageway")

    lane_rows, curb_rows = [], []
    for edge_id, is_rev, lanes, length_m, source, oneway, wkt, tags in rows:
        tags = tags or {}
        turns, widths, w_each = lane_widths(tags, lanes, is_rev)
        bikes = _split(pick(tags, "bicycle:lanes", is_rev))
        psvs = _split(pick(tags, "psv:lanes", is_rev))
        buses = _split(pick(tags, "bus:lanes", is_rev))     # bus lanes are tagged either way
        n = len(w_each)
        half = sum(w_each) / 2.0
        gap = gaps.get(edge_id)
        # placement says where the line really is; a two-way edge only in right-hand traffic, where
        # lane 1 is the leftmost lane (left-hand two-way lanes are numbered from the other side)
        place = _placement(pick(tags, "placement", is_rev), w_each) if (oneway or side_sign < 0) else None
        run = 0.0
        for i in range(n):
            u = uses
            if i < len(bikes) and bikes[i] in ("designated", "yes"):
                u = "bike"
            elif (i < len(psvs) and psvs[i] in ("designated", "yes")) or \
                    (i < len(buses) and buses[i] in ("designated", "yes")):
                u = "bus"
            width = _num(widths[i]) if i < len(widths) else None
            turn = turns[i] if i < len(turns) and turns[i] not in ("", "none") else None
            if place is not None:                            # OSM placement: from where the line lies
                off_m = place - (run + w_each[i] / 2.0)
            elif oneway and gap is not None:                 # one side of a road mapped as 2 ways
                off_m = side_sign * (run + w_each[i] / 2.0 - gap / 2.0)
            elif oneway:                                     # one-way: lanes centered on the carriageway
                off_m = half - (run + w_each[i] / 2.0)
            else:                                            # two-way: this direction's lanes on its travel side
                off_m = side_sign * (run + w_each[i] / 2.0)
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
      lane_id, link_id, lane_num, allowed_uses, r_barrier::VARCHAR AS r_barrier,
      l_barrier::VARCHAR AS l_barrier, width::DOUBLE AS width, turn::VARCHAR AS turn,
      {geom_sel} AS geom FROM _lane_df""")
    con.unregister("_lane_df")

    curb_df = pd.DataFrame(curb_rows, columns=[
        "curb_seg_id", "link_id", "ref_node_id", "start_lr", "end_lr", "regulation", "width"])
    con.register("_curb_df", curb_df)
    con.execute(f"CREATE TABLE {sch}.curb_seg AS SELECT * FROM _curb_df")
    con.unregister("_curb_df")


def _meso_modes_present(con):
    """Modes that have a gmns_<mode> schema (a built GMNS network) to derive a meso net from."""
    return [r[0].replace("gmns_", "") for r in con.execute(
        "SELECT DISTINCT schema_name FROM duckdb_tables() "
        "WHERE schema_name LIKE 'gmns_%' AND table_name = 'link' ORDER BY schema_name").fetchall()]


_MICRO_COLS = ("link_id, from_node_id, to_node_id, dir_flag, length, lanes, width, free_speed, "
               "facility_type, allowed_uses, geometry, geom, macro_link_id, meso_link_id, lane_no, "
               "cell_type, mvmt_txt_id, ctrl_type")


def to_micro(gmns_db, modes=None, cell_length_m=7.0):
    """Build a **microscopic (cell-based)** network into an existing GMNS DuckDB, from its
    ``gmns_<mode>`` lane + movement tables. Writes a ``micro_<mode>`` schema (``micro_node`` +
    ``micro_link``) per mode. Three kinds of micro link: **normal** cells (each lane sliced into
    ``cell_length_m`` pieces), **lane_change** connectors (adjacent lanes), and **movement** turn
    connectors (inbound lane end → outbound lane start, reusing the smooth Bézier). Ids are stable,
    reversible composites; ``macro_link_id`` / ``lane_no`` kept as columns. Default mode: driving."""
    import duckdb

    con = duckdb.connect(gmns_db)
    con.execute("INSTALL spatial; LOAD spatial;")
    have = _meso_modes_present(con)
    chosen = list(modes) if modes else ["driving"]
    unknown = [m for m in chosen if m not in have]
    if unknown:
        con.close()
        raise ValueError(f"mode(s) {unknown} have no gmns_<mode> schema (present: {have or 'none'})")

    result = {}
    for mode in chosen:
        g, mc = f"gmns_{mode}", f"micro_{mode}"
        cl = cell_length_m
        con.execute(f"DROP SCHEMA IF EXISTS {mc} CASCADE")
        con.execute(f"CREATE SCHEMA {mc}")
        # micro nodes: one per (lane, cell boundary k = 0..nc)
        con.execute(f"""CREATE TABLE {mc}.micro_node AS
          WITH L AS (SELECT lane_id, link_id, lane_num, geom,
                            GREATEST(1, CEIL(ST_Length(geom) * 111320.0 / {cl}))::BIGINT AS nc
                     FROM {g}.lane WHERE geom IS NOT NULL)
          SELECT lane_id, k AS cell_k, lane_id || '@' || k AS node_id,
                 ST_X(ST_LineInterpolatePoint(geom, k::DOUBLE / nc)) AS x_coord,
                 ST_Y(ST_LineInterpolatePoint(geom, k::DOUBLE / nc)) AS y_coord,
                 link_id AS macro_link_id, lane_num AS lane_no,
                 ST_LineInterpolatePoint(geom, k::DOUBLE / nc) AS geom
          FROM L, range(nc + 1) AS s(k)""")
        # normal (cell) micro links
        con.execute(f"""CREATE TABLE {mc}.micro_link AS
          WITH L AS (SELECT la.lane_id, la.link_id, la.lane_num, la.allowed_uses, la.width, la.geom,
                            lk.free_speed, lk.facility_type,
                            GREATEST(1, CEIL(ST_Length(la.geom) * 111320.0 / {cl}))::BIGINT AS nc
                     FROM {g}.lane la JOIN {g}.link lk ON lk.link_id = la.link_id
                     WHERE la.geom IS NOT NULL)
          SELECT 'C' || lane_id || '#' || k AS link_id, lane_id || '@' || k AS from_node_id,
                 lane_id || '@' || (k + 1) AS to_node_id, 1 AS dir_flag,
                 (ST_Length(ST_LineSubstring(geom, k::DOUBLE/nc, LEAST(1.0,(k+1.0)/nc))) * 111320)::DOUBLE AS length,
                 1 AS lanes, width, free_speed, facility_type, allowed_uses,
                 ST_AsText(ST_LineSubstring(geom, k::DOUBLE/nc, LEAST(1.0,(k+1.0)/nc))) AS geometry,
                 ST_LineSubstring(geom, k::DOUBLE/nc, LEAST(1.0,(k+1.0)/nc)) AS geom,
                 link_id AS macro_link_id, 'M' || link_id AS meso_link_id, lane_num AS lane_no,
                 'normal' AS cell_type, NULL::VARCHAR AS mvmt_txt_id, NULL::VARCHAR AS ctrl_type
          FROM L, range(nc) AS s(k)""")
        # lane-change micro links: adjacent lanes of a link, one cell forward (both directions)
        con.execute(f"""INSERT INTO {mc}.micro_link ({_MICRO_COLS})
          SELECT 'H' || a.node_id || '-' || b.node_id, a.node_id, b.node_id, 1,
                 ST_Distance(a.geom, b.geom) * 111320, 1, NULL::DOUBLE, NULL::DOUBLE, 'lane_change',
                 NULL::VARCHAR, ST_AsText(ST_MakeLine(a.geom, b.geom)), ST_MakeLine(a.geom, b.geom),
                 a.macro_link_id, 'M' || a.macro_link_id, NULL::INTEGER, 'lane_change',
                 NULL::VARCHAR, NULL::VARCHAR
          FROM {mc}.micro_node a JOIN {mc}.micro_node b
            ON a.macro_link_id = b.macro_link_id AND abs(a.lane_no - b.lane_no) = 1
               AND b.cell_k = a.cell_k + 1""")
        # movement (turn) micro links: inbound lane end -> outbound lane start, smooth Bézier
        con.execute(f"""INSERT INTO {mc}.micro_link ({_MICRO_COLS})
          WITH ie AS (SELECT macro_link_id AS lk, lane_no, arg_max(node_id, cell_k) AS node
                      FROM {mc}.micro_node GROUP BY macro_link_id, lane_no),
               oe AS (SELECT macro_link_id AS lk, lane_no, arg_min(node_id, cell_k) AS node
                      FROM {mc}.micro_node GROUP BY macro_link_id, lane_no)
          SELECT 'X' || ie.node || '-' || oe.node, ie.node, oe.node, 1,
                 ST_Length(ST_GeomFromText(m.geometry)) * 111320, 1, NULL::DOUBLE, il.free_speed,
                 'connector', m.allowed_uses, m.geometry, ST_GeomFromText(m.geometry),
                 NULL::BIGINT, 'M' || m.ib_link_id, NULL::INTEGER, 'movement', m.mvmt_code, m.ctrl_type
          FROM {g}.movement m          -- one connector per lane pair of the movement's ranges, in order
          JOIN ie ON ie.lk = m.ib_link_id
            AND ie.lane_no BETWEEN COALESCE(m.start_ib_lane, 1) AND COALESCE(m.end_ib_lane, m.start_ib_lane, 1)
          JOIN oe ON oe.lk = m.ob_link_id
            AND oe.lane_no = COALESCE(m.start_ob_lane, 1) + ie.lane_no - COALESCE(m.start_ib_lane, 1)
          JOIN {g}.link il ON il.link_id = m.ib_link_id
          WHERE m.geometry IS NOT NULL""")
        r = {
            "micro_node": con.execute(f"SELECT count(*) FROM {mc}.micro_node").fetchone()[0],
            "cell": con.execute(f"SELECT count(*) FROM {mc}.micro_link WHERE cell_type='normal'").fetchone()[0],
            "lane_change": con.execute(f"SELECT count(*) FROM {mc}.micro_link WHERE cell_type='lane_change'").fetchone()[0],
            "movement": con.execute(f"SELECT count(*) FROM {mc}.micro_link WHERE cell_type='movement'").fetchone()[0],
        }
        r["micro_link"] = r["cell"] + r["lane_change"] + r["movement"]
        result[mode] = r
        logger.info(f"MICRO[{mode}]: {r['micro_node']:,} nodes, {r['micro_link']:,} links "
                    f"({r['cell']:,} cell + {r['lane_change']:,} lane-change + {r['movement']:,} turn) -> {mc}")
    con.close()
    return result


def to_meso(gmns_db, modes=None, trim_m=6.0):
    """Build a mesoscopic (lane-level) network into an existing GMNS DuckDB, from its gmns_<mode>
    tables. Writes a ``meso_<mode>`` schema (``meso_node`` + ``meso_link``) per mode.

    A meso net is two kinds of link: a **normal** (section) link per macro link, and a **movement**
    (connector) link per legal turn (from ``movement``, so turn restrictions are honoured). Lane→turn
    assignment (``start_ib_lane``/``end_ib_lane``) comes from the ``turn:lanes``-derived ``lane.turn``.
    Ids are stable, reversible composites (``M<edge_id>`` / ``X<from>-<to>``); ``macro_link_id`` is kept
    as a column. Default mode: ``driving`` (meso is a vehicular-lane construct). Returns per-mode counts.
    """
    import duckdb

    con = duckdb.connect(gmns_db)                            # writable — adds meso_<mode> schemas
    con.execute("INSTALL spatial; LOAD spatial;")
    have = _meso_modes_present(con)
    chosen = list(modes) if modes else ["driving"]
    unknown = [m for m in chosen if m not in have]
    if unknown:
        con.close()
        raise ValueError(f"mode(s) {unknown} have no gmns_<mode> schema (present: {have or 'none'})")

    result = {}
    for mode in chosen:
        g, ms = f"gmns_{mode}", f"meso_{mode}"
        con.execute(f"DROP SCHEMA IF EXISTS {ms} CASCADE")
        con.execute(f"CREATE SCHEMA {ms}")
        _build_meso_nodes(con, g, ms, trim_m)
        _build_meso_links(con, g, ms, trim_m)
        r = {
            "meso_node": con.execute(f"SELECT count(*) FROM {ms}.meso_node").fetchone()[0],
            "normal": con.execute(
                f"SELECT count(*) FROM {ms}.meso_link WHERE meso_type='normal'").fetchone()[0],
            "movement": con.execute(
                f"SELECT count(*) FROM {ms}.meso_link WHERE meso_type='movement'").fetchone()[0],
        }
        r["meso_link"] = r["normal"] + r["movement"]
        result[mode] = r
        logger.info(f"MESO[{mode}]: {r['meso_node']:,} nodes, {r['meso_link']:,} links "
                    f"({r['normal']:,} section + {r['movement']:,} connector) -> {ms}")
    con.close()
    return result


def _build_meso_nodes(con, g, ms, trim_m):
    """Two meso nodes per macro link — upstream/downstream, inset ``trim_m`` metres from each end."""
    con.execute(f"""CREATE TABLE {ms}.meso_node AS
      WITH L AS (SELECT link_id, from_node_id, to_node_id, geom,
                        LEAST(0.35, {trim_m} / GREATEST("length", 0.1)) AS tf FROM {g}.link)
      SELECT link_id::VARCHAR || 'u' AS node_id,
             ST_X(ST_LineInterpolatePoint(geom, tf)) AS x_coord,
             ST_Y(ST_LineInterpolatePoint(geom, tf)) AS y_coord,
             from_node_id AS macro_node_id, link_id AS macro_link_id,
             ST_LineInterpolatePoint(geom, tf) AS geom
      FROM L
      UNION ALL
      SELECT link_id::VARCHAR || 'd',
             ST_X(ST_LineInterpolatePoint(geom, 1 - tf)),
             ST_Y(ST_LineInterpolatePoint(geom, 1 - tf)),
             to_node_id, link_id, ST_LineInterpolatePoint(geom, 1 - tf)
      FROM L""")


def _build_meso_links(con, g, ms, trim_m):
    """Normal (section) meso links + movement (connector) meso links."""
    con.execute(f"""CREATE TABLE {ms}.meso_link AS
      WITH L AS (SELECT link_id, geom, "length" AS len, lanes, free_speed, facility_type, allowed_uses,
                        LEAST(0.35, {trim_m} / GREATEST("length", 0.1)) AS tf FROM {g}.link)
      SELECT 'M' || link_id::VARCHAR AS link_id,
             link_id::VARCHAR || 'u' AS from_node_id, link_id::VARCHAR || 'd' AS to_node_id,
             1 AS dir_flag, (len * (1 - 2 * tf))::DOUBLE AS length, lanes::INTEGER AS lanes,
             NULL::DOUBLE AS capacity, free_speed, facility_type, allowed_uses,
             ST_AsText(ST_LineSubstring(geom, tf, 1 - tf)) AS geometry,
             ST_LineSubstring(geom, tf, 1 - tf) AS geom,
             'normal' AS meso_type, link_id AS macro_link_id, NULL::VARCHAR AS movement_id,
             NULL::VARCHAR AS mvmt_txt_id, NULL::INTEGER AS start_ib_lane, NULL::INTEGER AS end_ib_lane,
             NULL::VARCHAR AS ctrl_type
      FROM L""")

    con.execute(f"""INSERT INTO {ms}.meso_link
      (link_id, from_node_id, to_node_id, dir_flag, length, lanes, capacity, free_speed,
       facility_type, allowed_uses, geometry, geom, meso_type, macro_link_id, movement_id,
       mvmt_txt_id, start_ib_lane, end_ib_lane, ctrl_type)
      SELECT 'X' || m.ib_link_id::VARCHAR || '-' || m.ob_link_id::VARCHAR,
             m.ib_link_id::VARCHAR || 'd', m.ob_link_id::VARCHAR || 'u', 1,
             (ST_Length(ST_GeomFromText(m.geometry)) * 111320.0)::DOUBLE,   -- smooth Bézier length
             COALESCE(m.end_ib_lane - m.start_ib_lane + 1, 1)::INTEGER, NULL::DOUBLE, il.free_speed,
             'connector', m.allowed_uses,
             m.geometry, ST_GeomFromText(m.geometry),          -- reuse the movement's smooth connector
             'movement', NULL::BIGINT, m.mvmt_id,
             CASE m.type WHEN 'left' THEN 'L' WHEN 'right' THEN 'R' WHEN 'uturn' THEN 'U'
                         ELSE 'T' END,
             m.start_ib_lane, m.end_ib_lane, m.ctrl_type          -- the movement's own lanes
      FROM {g}.movement m
      JOIN {ms}.meso_node nd ON nd.node_id = m.ib_link_id::VARCHAR || 'd'
      JOIN {ms}.meso_node nu ON nu.node_id = m.ob_link_id::VARCHAR || 'u'
      JOIN {g}.link il ON il.link_id = m.ib_link_id""")


def _dump_csv(con, modes, to_csv, schema_prefix="gmns_"):
    """COPY each GMNS table to a spec-standard CSV (dropping the non-spec geom/turn columns).
    Skips tables a schema doesn't have (e.g. the combined ``gmns_all`` carries only node/link/config/
    use_*)."""
    tables = ["config", "node", "link", "geometry", "lane", "movement",
              "use_definition", "use_group", "signal_controller", "curb_seg"]
    for mode in modes:
        sch = f"{schema_prefix}{mode}"
        d = os.path.join(to_csv, mode) if len(modes) > 1 else to_csv
        os.makedirs(d, exist_ok=True)
        for t in tables:
            if con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name = ? "
                           "AND table_name = ?", [sch, t]).fetchone()[0] == 0:
                continue
            excl = _CSV_EXCLUDE.get(t)
            sel = f"* EXCLUDE ({', '.join(excl)})" if excl else "*"
            path = os.path.join(d, f"{t}.csv")
            con.execute(f"COPY (SELECT {sel} FROM {sch}.{t}) TO '{path}' (HEADER, DELIMITER ',')")
