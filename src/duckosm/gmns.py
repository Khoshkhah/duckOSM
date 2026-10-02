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
from collections import Counter, defaultdict
import os
import re

logger = logging.getLogger("duckosm")

# duckOSM mode schema -> GMNS use token, and use -> (persons_per_vehicle, pce)
_MODE_USES = {"driving": "auto", "walking": "walk", "cycling": "bike"}
_USE_DEF = {"auto": (1.0, 1.0), "walk": (1.0, 0.0), "bike": (1.0, 0.2), "bus": (25.0, 2.0)}
_DEFAULT_BIKE_LANE_W = 1.5       # metres, an on-road bike lane without a width tag
_DEFAULT_BIKE_W = 1.5           # metres, an on-road bike lane without a width (lanestyle draws the same)
_DEFAULT_WALK_W = 2.0           # metres, a footpath lane without a width (lanestyle draws the same)
_DEFAULT_LANE_W = 3.25          # metres, when width:lanes is absent (used for lane offset spacing)
_LANE_W_BY_CLASS = {"service": 2.5, "residential": 3.0, "living_street": 3.0, "unclassified": 3.0}   # narrower roads, same default


def _default_lane_w(highway):
    return _LANE_W_BY_CLASS.get((highway or "").split(";")[0].removesuffix("_link"), _DEFAULT_LANE_W)
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

def _len_m(g):
    """SQL: the length in metres of a lon/lat geometry, on the ellipsoid. (A degree of longitude is shorter
    than one of latitude, so ``ST_Length(g) * 111320`` is wrong everywhere but along a meridian.)"""
    return f"ST_Length_Spheroid(ST_FlipCoordinates({g}))"


def _src_cols(con, mode, table):
    """The columns of the source db's ``<mode>.<table>``."""
    return {c for (c,) in con.execute(
        "SELECT column_name FROM duckdb_columns() WHERE database_name = 's' AND schema_name = ? "
        "AND table_name = ?", [mode, table]).fetchall()}


# OSM `cycleway` / `sidewalk` -> GMNS's category lists (spec/shared_categories.json); NULL stays NULL.
_BIKE_FACILITY = ("CASE WHEN {v} IS NULL THEN NULL ELSE CASE {v} WHEN 'lane' THEN 'unseparated bike lane' WHEN 'track' THEN 'separated bike lane' "
                  "WHEN 'share_busway' THEN 'shared lane' WHEN 'shared_lane' THEN 'shared lane' "
                  "WHEN 'opposite' THEN 'counter-flow bike lane' WHEN 'opposite_lane' THEN 'counter-flow bike lane' "
                  "WHEN 'shoulder' THEN 'paved shoulder' WHEN 'no' THEN 'none' WHEN 'none' THEN 'none' "
                  "ELSE 'other' END END")
_PED_FACILITY = ("CASE WHEN {v} IS NULL THEN NULL ELSE CASE {v} WHEN 'both' THEN 'sidewalk' WHEN 'left' THEN 'sidewalk' WHEN 'right' THEN 'sidewalk' "
                 "WHEN 'yes' THEN 'sidewalk' WHEN 'separate' THEN 'offstreet_path' WHEN 'no' THEN 'none' "
                 "WHEN 'none' THEN 'none' ELSE 'unknown' END END")

# Roads cars cannot use: in the walking / cycling schemas their `lanes` and `capacity` stay empty, since the
# standard defines both for motor vehicles ("uncapacitated" bike and foot links; link.lanes excludes bike lanes)
_CROSSING_ALONG = 0.6            # a footway=crossing link that runs along a road for this share of its length, and is at least _CROSSING_MIN_M long, is a footpath OSM tagged by mistake
_CROSSING_MIN_M = 10.0           # (a crossing of a side street also runs along the main road, but is short: Monaco's crossings are 5 m at the median)
_ALONG_NEAR_M = 8.0              # a footpath point counts as along a road when its edge is within this many metres of the road's edge
_NON_MOTOR = ("footway", "path", "cycleway", "steps", "pedestrian", "bridleway", "corridor", "platform")

# GMNS tables that carry a non-spec column for the DuckDB output — dropped for --to-csv fidelity
_CSV_EXCLUDE = {"node": ["geom"], "link": ["geom", "osm_id", "edge_ref", "footway", "crossing", "crossing_markings", "along_link_id", "along_mode", "along_gap_m", "along_kind", "bridge", "tunnel", "layer"], "geometry": ["geom"], "lane": ["geom", "turn"],
    "location": ["osm_id", "name", "geom"], "zone": ["geom"],
    "signal_controller": ["node_id", "control_type"]}


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


def _chain_runs(info):
    """Runs of pieces (docs/design/gmns_lane_runs.md): consecutive edges joined end to start, with the
    same ``oneway`` and lane widths, that are one road: pieces of one OSM way and direction, pieces of
    a roundabout (OSM draws one as several ways), or a way going on into another way of the same
    name straight ahead (within 30 degrees). Each joint one-to-one. ``info``: ``edge_id -> (osm_id,
    is_reverse, source, target, oneway, lane widths, name, roundabout, heading in, heading out)``.
    Returns ``(runs, where, closed)``: the runs as edge lists in order, ``edge_id -> (run index,
    position)``, and the set of run indexes that are rings (the last piece goes on into the first)."""
    from collections import Counter, defaultdict

    at_src, nxt = defaultdict(list), {}
    for e, d in info.items():
        at_src[d[2]].append(e)

    def one_road(a, b):
        ea, eb = info[a], info[b]
        if ea[4] != eb[4] or ea[5] != eb[5]:
            return False
        if (ea[0], ea[1]) == (eb[0], eb[1]) or (ea[7] and eb[7]):
            return True
        turn = abs((eb[8] - ea[9] + 180) % 360 - 180)
        return bool(ea[6]) and ea[6] == eb[6] and turn < 30

    for e, d in info.items():
        c = [f for f in at_src.get(d[3], []) if f != e and one_road(e, f)]
        if len(c) == 1:
            nxt[e] = c[0]
    taken = Counter(nxt.values())
    nxt = {e: f for e, f in nxt.items() if taken[f] == 1}
    prv = {f: e for e, f in nxt.items()}
    runs, seen = [], set()
    for start in [e for e in info if e not in prv] + list(info):   # chain starts first, then any cycle
        if start in seen:
            continue
        run, x = [], start
        while x is not None and x not in seen:
            seen.add(x)
            run.append(x)
            x = nxt.get(x)
        runs.append(run)
    closed = {i for i, run in enumerate(runs) if len(run) > 1 and nxt.get(run[-1]) == run[0]}
    return runs, {e: (i, k) for i, run in enumerate(runs) for k, e in enumerate(run)}, closed


def _run_lane_wkts(piece_wkts, offs, closed=False):
    """The lanes of a run: each lane offset once from the run's merged line (so a bend is one curve
    and the pieces' lanes meet exactly), then cut back into the pieces at the joints (the lane point
    nearest each joint). A ``closed`` run (a roundabout) is offset by buffering the ring it encloses
    (GEOS's offset curve of a closed line is unreliable on the inside), with the seam in the middle
    of the first piece, which is stitched back together. Returns
    ``[[lane wkt, ...] per piece]``, or None when the geometry won't do (a joint folding back on
    the offset curve): the caller then falls back to per-piece offsets."""
    from shapely import wkt as _w
    from shapely.geometry import LinearRing, LineString, Point, Polygon
    from shapely.ops import substring

    lines = [_w.loads(w) for w in piece_wkts]
    if any(ln.is_empty or ln.geom_type != "LineString" for ln in lines):
        return None
    n = len(lines)
    x0, y0 = lines[0].coords[0]
    kx, M = (math.cos(math.radians(y0)) or 1.0), 111320.0
    loc = [LineString([((x - x0) * kx * M, (y - y0) * M) for x, y in ln.coords]) for ln in lines]
    ring = closed and n > 1 and loc[0].coords[0] == loc[-1].coords[-1] or (
        closed and n > 1 and Point(loc[0].coords[0]).distance(Point(loc[-1].coords[-1])) < 0.01)
    if ring:                                     # seam at the first piece's midpoint
        half = loc[0].length / 2
        head, tail = substring(loc[0], 0, half), substring(loc[0], half, loc[0].length)
        order = [tail] + loc[1:] + [head]
    else:
        order = loc
    coords, joints = list(order[0].coords), []
    for ln in order[1:]:
        c = list(ln.coords)
        joints.append(Point(coords[-1]))
        if Point(c[0]).distance(Point(coords[-1])) < 0.01:
            c = c[1:]
        coords += c
    merged = LineString(coords)
    out = [[] for _ in range(n)]
    for off in offs:
        if abs(off) <= 1e-6:
            line = merged
        elif ring:                               # the ring's polygon, buffered in or out
            poly = Polygon(coords)
            if not poly.is_valid or poly.area < 1.0:
                return None
            ccw = LinearRing(coords).is_ccw
            buf = poly.buffer(-abs(off) if (off > 0) == ccw else abs(off))    # left (+) is inside a ccw ring
            if buf.is_empty or buf.geom_type != "Polygon":
                return None
            bd = list(buf.exterior.coords)
            if LinearRing(bd).is_ccw != ccw:
                bd = bd[::-1]
            line = LineString(bd)
            s0 = line.project(Point(coords[0]))  # start at the seam
            line = LineString(list(substring(line, s0, line.length).coords) + list(substring(line, 0, s0).coords)[1:])
        else:
            line = merged.offset_curve(off)
        if line.is_empty:
            return None
        if line.geom_type == "MultiLineString":
            line = max(line.geoms, key=lambda s: s.length)
        ds = [0.0] + [line.project(j) for j in joints] + [line.length]
        if any(ds[i + 1] - ds[i] < 0.3 for i in range(len(ds) - 1)):
            return None
        segs = [substring(line, ds[i], ds[i + 1]) for i in range(len(ds) - 1)]
        if ring:                                 # the first piece: its head (at the end) + its tail (at the start)
            segs = [LineString(list(segs[-1].coords) + list(segs[0].coords)[1:])] + segs[1:-1]
        for i, seg in enumerate(segs):
            if seg.geom_type != "LineString" or len(seg.coords) < 2:
                return None
            out[i].append(LineString([(x / (kx * M) + x0, y / M + y0) for x, y in seg.coords]).wkt)
    return out


def _paired_gaps(edges, side_sign, step_m=2.0, profiles=None):
    """One-way edges placed as one side of a two-way road (docs/design/gmns_paired_carriageways.md).

    ``edges``: ``(edge_id, wkt, half_width_m)`` of the one-way edges. Returns ``{edge_id: d}``, the
    median gap in metres between an edge's centre line and its partner(s): one-way edges running the
    opposite way on its inner side (left for right-hand traffic) closer than ``half_A + half_B``,
    alongside it for at least three samples (the profile, below, takes the gap back to the plain placement where it is not). Sampled every ``step_m`` metres.

    ``profiles`` (a dict, filled in): ``{edge_id: [(s, g), ...]}``, the gap ``g`` at the distance ``s`` along the
    edge at every sample (docs/design/gmns_paired_carriageways.md, step 2): where no partner is alongside it is
    ``2 * half``, the plain one-way placement, and it never exceeds that; smoothed over three samples."""
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
        found, samples, prof = [], 0, []
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
            prof.append((s, best if best is not None else 2 * half[i]))
        if len(found) >= 3:                              # alongside for 6 m or more; the profile fades it where it is not
            gaps[edges[i][0]] = statistics.median(found)
            if profiles is not None:
                g = [x for _, x in prof]
                profiles[edges[i][0]] = [(prof[k][0], sum(g[max(k - 1, 0):k + 2]) / len(g[max(k - 1, 0):k + 2]))
                                         for k in range(len(prof))]
    return gaps


def _vary_wkt(lane_wkt, run_wkt, profile, g0, side_sign, step_m=2.0):
    """A paired edge's lane line, moved so its direction's lanes start from the *local* midline: each vertex goes along the
    lane's left normal by ``-side * (g(s) - g0) / 2``, ``g(s)`` the gap at its position along the run (``profile``), ``g0`` the
    median gap the line was placed with. The line is cut into ``step_m`` pieces first. Returns lon/lat WKT, the input unchanged
    on any failure."""
    import numpy as np
    import shapely
    from shapely import wkt as _w
    from shapely.geometry import LineString

    try:
        lane, run = _w.loads(lane_wkt), _w.loads(run_wkt)
        x0, y0 = run.coords[0]
        kx, M = (math.cos(math.radians(y0)) or 1.0), 111320.0
        loc = lambda g: LineString([((x - x0) * kx * M, (y - y0) * M) for x, y in g.coords])    # noqa: E731
        ln, rn = shapely.segmentize(loc(lane), step_m), loc(run)
        pts = np.array(ln.coords)
        if len(pts) < 2 or len(profile) < 2:
            return lane_wkt
        S, G = zip(*profile)
        tang = np.gradient(pts, axis=0)
        norm = np.hypot(tang[:, 0], tang[:, 1])
        norm[norm == 0] = 1.0
        left = np.stack([-tang[:, 1] / norm, tang[:, 0] / norm], axis=1)
        g = np.interp([rn.project(shapely.Point(p)) for p in pts], S, G)
        pts = pts + left * (-side_sign * (g - g0) / 2.0)[:, None]
        return LineString([(x / (kx * M) + x0, y / M + y0) for x, y in pts]).wkt
    except Exception:
        return lane_wkt


def to_gmns(source_db, out_path, modes=None, to_csv=None, lane_geometry=True, combined=False,
            drive_side="right", pair_carriageways=True, csv_extensions=False, gtfs=None, gtfs_max_m=30.0,
            walk_frame=False, walk_clearance_m=0.0):
    """Extract a built duckOSM db to a standalone GMNS DuckDB.

    Parameters
    ----------
    source_db : path to a built duckOSM ``.duckdb`` (opened read-only).
    out_path : path of the GMNS ``.duckdb`` to create (overwritten if it exists).
    modes : mode schemas to extract (default: every mode present).
    to_csv : if set, also dump spec-standard GMNS CSVs into this directory (per-mode subfolders when
        more than one mode).
    lane_geometry : compute a per-lane offset ``geom`` for lane-level rendering (needs shapely).
    walk_frame : a sidewalk (``footway=sidewalk``, its ``parent_link_id`` the road it runs along) takes its place
        from that road's cross-section: just outside the road's kerb-side lane, not from its own OSM line
        (docs/design/gmns_walking_frame.md); ``walk_clearance_m`` leaves a gap to the kerb. Only mapped sidewalks move:
        no footpath is put where OSM has none.
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
    area = re.sub(r"_gmns$", "", name)               # the zone's name when the area has no boundary
    result = {"path": out_path, "csv": None, "modes": {}}
    stops = None
    if gtfs:
        from duckosm.gtfs import read_stops
        stops = read_stops(gtfs)
        logger.info(f"GTFS: {len(stops)} stops")
    for mode in chosen:
        sch = f"gmns_{mode}"
        uses = _MODE_USES.get(mode, mode)
        con.execute(f"CREATE SCHEMA {sch}")
        _build_fixed(con, sch, name, mode, uses)
        _build_node(con, sch, mode)
        _infer_lanes(con, mode, has_raw)
        _build_link(con, sch, mode, uses, has_raw)
        _build_geometry(con, sch, mode)
        for col, typ in (("along_link_id", "BIGINT"), ("along_mode", "VARCHAR"), ("along_gap_m", "DOUBLE"), ("along_kind", "VARCHAR")):
            con.execute(f"ALTER TABLE {sch}.link ADD COLUMN {col} {typ}")      # filled for a sidewalk: _sidewalk_parents
        con.execute(f"CREATE TABLE {sch}.link_along(link_id BIGINT, along_link_id BIGINT, along_mode VARCHAR, "
                    f"covered_m DOUBLE, gap_m DOUBLE)")        # every road a footpath runs along (docs/exports/gmns_tables.md); empty where no footpaths
        along = _sidewalk_parents(con, sch, has_raw, drive_side) if mode != "driving" else {}
        _build_lane_curb(con, sch, mode, uses, has_raw, lane_geometry, drive_side,
                         pair_carriageways)  # before movement
        if walk_frame and mode != "driving" and lane_geometry:
            _walk_frame(con, sch, drive_side, along, walk_clearance_m)  # before the lane connectors
            _walk_kerb(con, sch, walk_clearance_m)
            _walk_join(con, sch)
        _build_movement(con, sch, mode, uses, drive_side)
        if lane_geometry and mode == "driving":         # lane ends + connectors, after the lane ranges. Driving only: a footway has no turning
            _build_lane_connectors(con, sch)            # path through a junction (OSM has none), its lane runs to its node and meets the next there
        if mode == "driving":
            from duckosm.crossings import build_crossings
            build_crossings(con, sch, has_raw)          # zebra crossings on the driving lanes (docs/design/gmns_crossings.md)
        _build_signal_controller(con, sch)
        _build_location(con, sch, mode, has_raw, create_empty=stops is not None)
        if stops is not None:
            _add_transit_stops(con, sch, mode, stops, drive_side, gtfs_max_m)
        _apply_signs(con, sch, mode, has_raw)
        _node_types(con, sch, has_raw)
        _build_zone(con, sch, area)
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
        _dump_csv(con, chosen, to_csv, csv_extensions=csv_extensions)
        if result["combined"]:
            _dump_csv(con, ["all"], os.path.join(to_csv, "combined"), schema_prefix="gmns_", csv_extensions=csv_extensions)
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
      ('{name}_all', 'meter', 'meter', 'kmh', 'EPSG:4326', 'wkt', NULL::VARCHAR, 0.97::DOUBLE, 'integer')
    ) t(dataset_name, short_length, long_length, speed, crs, geometry_field_format,
        currency, version_number, id_type)""")
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
             any_value(row_width) AS row_width, any_value(osm_id) AS osm_id, any_value(edge_ref) AS edge_ref, any_value(footway) AS footway, any_value(crossing) AS crossing, any_value(crossing_markings) AS crossing_markings, any_value(bridge) AS bridge,
             any_value(tunnel) AS tunnel, any_value(layer) AS layer, any_value(geom) AS geom
      FROM u GROUP BY link_id""")
    return {t: con.execute(f"SELECT count(*) FROM gmns_all.{t}").fetchone()[0]
            for t in ("node", "link")}


def _build_fixed(con, sch, name, mode, uses):
    """config + use_definition + use_group — the small fixed tables."""
    con.execute(f"""CREATE TABLE {sch}.config AS SELECT * FROM (VALUES
      ('{name}_{mode}', 'meter', 'meter', 'kmh', 'EPSG:4326', 'wkt', NULL::VARCHAR, 0.97::DOUBLE, 'integer')
    ) t(dataset_name, short_length, long_length, speed, crs, geometry_field_format,
        currency, version_number, id_type)""")
    # every use a lane can carry must be defined (spec: lane.allowed_uses): a driving network's lanes
    # are auto, bus or bike
    defined = [uses] + (["bus", "bike"] if mode == "driving" else [])
    rows = ", ".join(f"('{u}', {_USE_DEF.get(u, (1.0, 1.0))[0]}, {_USE_DEF.get(u, (1.0, 1.0))[1]})"
                     for u in defined)
    con.execute(f"""CREATE TABLE {sch}.use_definition AS SELECT use, persons_per_vehicle, pce,
      NULL::VARCHAR AS special_conditions, NULL::VARCHAR AS description
      FROM (VALUES {rows}) t(use, persons_per_vehicle, pce)""")
    con.execute(f"""CREATE TABLE {sch}.use_group AS SELECT
      '{mode}' AS use_group, '{uses}' AS uses, NULL::VARCHAR AS description""")


def _build_node(con, sch, mode):
    z = "n.ele::DOUBLE" if "ele" in _src_cols(con, mode, "nodes") else "NULL::DOUBLE"   # `duckosm elevation`
    con.execute(f"""CREATE TABLE {sch}.node AS SELECT
      n.node_id, NULL::VARCHAR AS name, ST_X(n.geom) AS x_coord, ST_Y(n.geom) AS y_coord,
      {z} AS z_coord, NULL::VARCHAR AS node_type,
      CASE WHEN sig.osm_id IS NOT NULL THEN 'signal' END AS ctrl_type,
      1::BIGINT AS zone_id, NULL::BIGINT AS parent_node_id, n.geom      -- every node is in the one zone
    FROM s.{mode}.nodes n LEFT JOIN _sig sig ON sig.osm_id = n.node_id
    WHERE n.geom IS NOT NULL""")


def _build_link(con, sch, mode, uses, has_raw):
    raw_join = "LEFT JOIN s.raw.ways w ON w.osm_id = abs(e.osm_id)" if has_raw else ""   # a virtual (negative) id is its OSM way with a minus: its tags are the way's
    bike = _BIKE_FACILITY.format(v="w.tags['cycleway']") if has_raw else "NULL::VARCHAR"
    ped = _PED_FACILITY.format(v="w.tags['sidewalk']") if has_raw else "NULL::VARCHAR"
    cols = {c for (c,) in con.execute(
        "SELECT column_name FROM duckdb_columns() WHERE database_name = 's' AND schema_name = ? "
        "AND table_name = 'edges'", [mode]).fetchall()}
    lvl = ", ".join(f"e.{c}" if c in cols else f"NULL::VARCHAR AS {c}" for c in ("bridge", "tunnel", "layer"))
    edge_ref = "e.edge_ref AS edge_ref" if "edge_ref" in cols else "NULL::VARCHAR AS edge_ref"     # older builds have none
    footway = "w.tags['footway'] AS footway" if has_raw else "NULL::VARCHAR AS footway"
    crossing = ("CASE WHEN w.tags['footway'] = 'crossing' THEN w.tags['crossing'] END AS crossing, "
                "CASE WHEN w.tags['footway'] = 'crossing' THEN w.tags['crossing:markings'] END AS crossing_markings") if has_raw \
        else "NULL::VARCHAR AS crossing, NULL::VARCHAR AS crossing_markings"
    # grade (%) from `duckosm elevation`'s end heights. Not on a bridge or in a tunnel: there the height is the
    # ground below or above, not the road; and the spec's limit is 100 %
    glen = _len_m("e.geometry")                         # the same length as `length`: rise over run
    structure = " OR ".join(f"COALESCE(e.{c}, 'no') NOT IN ('no', '')" for c in ("tunnel", "bridge") if c in cols) or "false"
    grade = ("NULL::DOUBLE" if not {"z_from", "z_to"} <= cols else
             f"CASE WHEN NOT ({structure}) AND abs(100.0 * (e.z_to - e.z_from) / NULLIF({glen}, 0)) <= 100 "
             f"THEN 100.0 * (e.z_to - e.z_from) / {glen} END")
    hw = "regexp_replace(split_part(e.highway, ';', 1), '_link$', '')"
    motor = "true" if mode == "driving" else f"{hw} NOT IN ({', '.join(repr(h) for h in _NON_MOTOR)})"
    con.execute(f"""CREATE TABLE {sch}.link AS SELECT
      e.edge_id AS link_id, e.name AS name, e.source AS from_node_id, e.target AS to_node_id,
      true AS directed, e.edge_id AS geometry_id, ST_AsText(e.geometry) AS geometry,
      NULL::BIGINT AS parent_link_id, 1 AS dir_flag,
      {glen} AS length,   -- the geometry's own length, metres
      {grade}::DOUBLE AS grade,
      e.highway AS facility_type,
      CASE WHEN {motor} THEN {_capacity_case(hw)}::DOUBLE END AS capacity,
      e.maxspeed_kmh AS free_speed, CASE WHEN {motor} THEN COALESCE(li.lanes, e.lanes) END AS lanes,
      {bike} AS bike_facility, {ped} AS ped_facility, NULL::VARCHAR AS parking,
      '{uses}' AS allowed_uses, NULL::DOUBLE AS toll, NULL::VARCHAR AS jurisdiction,
      NULL::DOUBLE AS row_width, abs(e.osm_id) AS osm_id, {edge_ref}, {footway}, {crossing}, {lvl}, e.geometry AS geom
    FROM s.{mode}.edges e {raw_join} LEFT JOIN _lanes_inf li ON li.edge_id = e.edge_id""")


def _sidewalk_parents(con, sch, has_raw, drive_side, max_gap_m=30.0, max_turn_deg=30.0, adjacent_m=5.0):
    """``link.parent_link_id`` of a sidewalk (an OSM way with ``footway=sidewalk``): the road link it runs
    along, as the standard's own example says ("for a sidewalk, this is the adjacent road"). The nearest
    road link within ``max_gap_m`` metres that is roughly parallel (``max_turn_deg``). A two-way road is two
    links with the same shape; the sidewalk's parent is the one it is on the kerb side of (its right-hand
    side with right-hand traffic). A sidewalk with no such road keeps no parent.

    Roads of ``gmns_driving`` are looked at too (a road without a sidewalk is not in the walking network at all):
    returns every footpath link -> the road it runs along, for ``_walk_frame``; ``parent_link_id`` is written only
    where the road is a link of this schema."""
    if not has_raw:
        return {}
    import numpy as np
    import shapely
    from shapely import wkt as _w
    from shapely.strtree import STRtree

    from duckosm.crossings import _level

    # a mapped sidewalk, and any other footpath (footway, path, pedestrian, cycleway: not a crossing, a link or steps) that lies right beside a road
    near_classes = ", ".join(repr(h) for h in _NON_MOTOR if h not in ("steps", "platform", "corridor"))
    sw = con.execute(f"""SELECT k.link_id, ST_AsText(k.geom), k.layer, k.bridge, k.tunnel,
                           CASE WHEN w.tags['footway'] = 'sidewalk' THEN 'sidewalk' ELSE 'adjacent' END, w.tags['footway'] = 'crossing', k.from_node_id, k.to_node_id
                         FROM {sch}.link k JOIN s.raw.ways w ON w.osm_id = abs(k.osm_id)
                         WHERE w.tags['footway'] = 'sidewalk' OR (COALESCE(w.tags['footway'], '') NOT IN ('link', 'sidewalk')
                           AND regexp_replace(split_part(k.facility_type, ';', 1), '_link$', '') IN ({near_classes}))
                         """).fetchall()
    if not sw:
        return {}
    nonroad = ", ".join(repr(h) for h in _NON_MOTOR)
    have_drive = sch != "gmns_driving" and con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name = 'gmns_driving' "
                                                       "AND table_name = 'link'").fetchone()[0] > 0
    own = set()
    roads = []
    if not have_drive:               # a walking-only build: its own road classes are the roads
        roads = con.execute(f"""SELECT link_id, ST_AsText(geom), layer, bridge, tunnel FROM {sch}.link WHERE
            regexp_replace(split_part(facility_type, ';', 1), '_link$', '') NOT IN ({nonroad})""").fetchall()
        own = {r[0] for r in roads}
    else:                            # a road is where cars drive: a road only people walk on (a service road closed to cars) is no road to run along
        roads = con.execute(f"""SELECT link_id, ST_AsText(geom), layer, bridge, tunnel FROM gmns_driving.link WHERE
            regexp_replace(split_part(facility_type, ';', 1), '_link$', '') NOT IN ({nonroad})""").fetchall()
        own = {r[0] for r in con.execute(f"SELECT link_id FROM {sch}.link").fetchall()} & {r[0] for r in roads}
    if not roads:
        return {}
    first = _w.loads(sw[0][1]).coords[0]
    scale = np.array([math.cos(math.radians(first[1])) * 111320.0, 111320.0])        # lon/lat -> metres, locally
    to_m = lambda w: shapely.transform(_w.loads(w), lambda c: c * scale)             # noqa: E731
    road_ids, road_geoms = [r[0] for r in roads], [to_m(r[1]) for r in roads]
    road_level = [_level(*r[2:]) for r in roads]            # a sidewalk runs along a road of its own level (not the one under or over it)
    half = {}                                           # a road's edge is far from its centre line: the nearest road is the nearest edge
    for g_ in (sch, "gmns_driving"):
        if not con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name = ? AND table_name = 'link'", [g_]).fetchone()[0]:
            continue
        links = con.execute(f"SELECT link_id, COALESCE(lanes, 1), facility_type, edge_ref FROM {g_}.link").fetchall()
        pair = Counter(e[:-1] for *_, e in links if e)  # a twin pair is one two-way road: its kerb is a whole direction's lanes away
        for k, n, ft, e in links:
            half.setdefault(k, n * _default_lane_w(ft) * (1.0 if e and pair[e[:-1]] > 1 else 0.5))
    tree = STRtree(road_geoms)

    def heading(g, t):                                  # the line's direction at distance t along it, radians
        a, b = g.interpolate(max(t - 1.0, 0.0)), g.interpolate(min(t + 1.0, g.length))
        return math.atan2(b.y - a.y, b.x - a.x)

    kerb = -1.0 if drive_side == "right" else 1.0       # the sidewalk's side of its road link: right = negative
    parents, along_rows = [], []

    def match_one(link_id, g, lv, kind, crossing, allowed=None):
        """Match one footpath to the road it runs along (and the route: every road along it for 4 m or more); ``allowed``: only these roads. Returns the road ids of its route."""
        level = _level(*lv)
        mid = g.interpolate(0.5, normalized=True)
        samples = [g.interpolate(d) for d in [*range(0, int(g.length), 2), g.length]]
        best, route = None, []
        for i in tree.query(g, predicate="dwithin", distance=max_gap_m):
            if road_level[i] != level or (allowed is not None and road_ids[i] not in allowed):
                continue
            r = road_geoms[i]
            hw = half.get(road_ids[i], 0.0)
            cov = 0                                      # how long the road runs along the footpath: samples near its edge and parallel to it
            for sp in samples:
                t = r.project(sp)
                if sp.distance(r.interpolate(t)) - hw > _ALONG_NEAR_M:
                    continue
                turn = abs(((heading(r, t) - heading(g, g.project(sp)) + math.pi / 2) % math.pi) - math.pi / 2)
                cov += math.degrees(turn) <= max_turn_deg
            if not cov or (crossing and cov < _CROSSING_ALONG * len(samples)):       # a crossing is square to the road: only one that runs along it is matched
                continue
            t = r.project(mid)
            p = r.interpolate(t)
            a, b = r.interpolate(max(t - 1.0, 0.0)), r.interpolate(min(t + 1.0, r.length))
            side = (b.x - a.x) * (mid.y - p.y) - (b.y - a.y) * (mid.x - p.x)         # > 0: left of the road link
            # the road along the longest stretch of it (a 5 m side road touching its end does not count), then the nearest edge, then the kerb side
            key = (-cov, round(max(g.distance(r) - hw, 0.0) * 2) / 2, 0 if side * kerb > 0 else 1)
            if cov >= 2 and (kind == "sidewalk" or g.distance(r) - hw - _DEFAULT_WALK_W / 2 <= adjacent_m):
                route.append((road_ids[i], cov * 2.0, g.distance(r)))        # a road it runs along for 4 m or more: one of the route
            if best is None or key < best[0]:
                best = (key, road_ids[i], g.distance(r), g.distance(r) - hw - _DEFAULT_WALK_W / 2)
        if best is None or not (kind == "sidewalk" or best[3] <= adjacent_m):    # an unmarked footpath only where it is right beside the road
            return set()
        parents.append((best[1], link_id, best[2], kind))
        for rid, cov_m, dist in route:
            along_rows.append((link_id, rid, sch.removeprefix("gmns_") if rid in own else "driving", cov_m, round(dist, 2)))
        return {rid for rid, _, _ in route} | {best[1]}

    at_node = defaultdict(set)                       # node -> the roads the matched footpaths that end there run along
    deferred = []                                    # short crossings: a crossing of a side street also runs along the main road, so one is matched only where it continues a matched footpath
    for link_id, w, *lv, kind, crossing, n0, n1 in sw:
        g = to_m(w)
        if g.length < 1.0:
            continue
        if crossing and g.length < _CROSSING_MIN_M:
            deferred.append((link_id, g, lv, kind, n0, n1))
            continue
        got = match_one(link_id, g, lv, kind, crossing)
        at_node[n0] |= got
        at_node[n1] |= got
    for link_id, g, lv, kind, n0, n1 in deferred:
        allowed = at_node.get(n0, set()) | at_node.get(n1, set())
        if allowed:
            match_one(link_id, g, lv, kind, True, allowed)
    if any(r in own for r, _, _, kd in parents if kd == "sidewalk"):
        con.executemany(f"UPDATE {sch}.link SET parent_link_id = ? WHERE link_id = ?", [(r, k) for r, k, _, kd in parents if r in own and kd == "sidewalk"])
    # duckOSM extension: the road it runs along, also where that road is only in gmns_driving (link_id is the same hash in
    # every schema), which of the two networks it is in, and how far the sidewalk's line is from the road's line
    if along_rows:
        con.executemany(f"INSERT INTO {sch}.link_along VALUES (?, ?, ?, ?, ?)", along_rows)
    if parents:
        con.executemany(f"UPDATE {sch}.link SET along_link_id = ?, along_mode = ?, along_gap_m = ?, along_kind = ? WHERE link_id = ?",
                        [(r, sch.removeprefix("gmns_") if r in own else "driving", round(d, 2), kd, k) for r, k, d, kd in parents])
    return {k: r for r, k, _, kd in parents if kd == "sidewalk"}      # only a mapped sidewalk may take its place from the road


def _walk_frame(con, sch, drive_side, along, clearance_m=0.0):
    """A footpath with a parent road takes its place from the road's cross-section (docs/design/gmns_walking_frame.md):
    its lane becomes an offset curve of the road's kerb-side lane, ``clearance_m`` outside its edge, cut to the stretch
    the footpath covers. A road in ``gmns_driving`` (its real lanes) is the frame where there is one. Only a footpath on
    its parent's kerb side moves.
    Lane geometry only: ids, lengths, lane numbers and movements stay."""
    import shapely
    from shapely import wkt as _w
    from shapely.geometry import LineString
    from shapely.ops import substring

    kids = [(r[0], r[1], along[r[1]], *r[2:]) for r in con.execute(f"""SELECT l.lane_id, l.link_id, ST_AsText(l.geom), l.width,
                           ST_AsText(k.geom) FROM {sch}.lane l JOIN {sch}.link k ON k.link_id = l.link_id
                           WHERE l.geom IS NOT NULL AND l.allowed_uses = 'walk'""").fetchall() if r[1] in along]
    if not kids:
        return
    have_drive = con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name = 'gmns_driving' "
                             "AND table_name = 'lane'").fetchone()[0] > 0
    kerb = -1.0 if drive_side == "right" else 1.0
    pick = max if drive_side == "right" else min                 # lane 1 is the leftmost: the kerb is the last lane on the right

    def roads(g):                                                # link -> (its lanes (num, wkt, width), its own line)
        lanes, line = {}, {r[0]: r[1] for r in con.execute(f"SELECT link_id, ST_AsText(geom) FROM {g}.link").fetchall()}
        for lk, num, w, wd in con.execute(f"SELECT link_id, lane_num, ST_AsText(geom), width FROM {g}.lane WHERE geom IS NOT NULL").fetchall():
            lanes.setdefault(lk, []).append((num, w, wd))
        return {lk: (v, line[lk]) for lk, v in lanes.items() if lk in line}
    frame = roads(sch)
    if have_drive:
        frame.update(roads("gmns_driving"))
    out, why = [], Counter()
    for lane_id, link_id, parent, cw, ws, lw in kids:
        if parent not in frame:
            why['no_road_lane'] += 1
            continue
        planes, pline = frame[parent]
        _, pw, wp = pick(plane for plane in planes)             # the kerb-side lane: its line and width
        wp, ws = wp or _DEFAULT_LANE_W, ws or _DEFAULT_WALK_W
        c, p, ln0, rl = _w.loads(cw), _w.loads(pw), _w.loads(lw), _w.loads(pline)
        if c.is_empty or p.is_empty or ln0.is_empty or rl.is_empty:
            why['degenerate'] += 1
            continue
        x0, y0 = p.coords[0]
        kx, M = math.cos(math.radians(y0)) or 1.0, 111320.0
        loc = lambda g: LineString([((x - x0) * kx * M, (y - y0) * M) for x, y in g.coords])    # noqa: E731
        cl, pl, ln, rn = loc(c), loc(p), loc(ln0), loc(rl)    # lane, road's lane, footpath's own line, road's line
        if cl.length < 1.0 or pl.length < 1.0 or ln.length < 1.0:
            why['short'] += 1
            continue
        mid = ln.interpolate(0.5, normalized=True)
        t = rn.project(mid)                                      # which side of the road link's own line it is on
        a, b, q = rn.interpolate(max(t - 1.0, 0.0)), rn.interpolate(min(t + 1.0, rn.length)), rn.interpolate(t)
        side = (b.x - a.x) * (mid.y - q.y) - (b.y - a.y) * (mid.x - q.x)
        if side * kerb <= 0:                                     # the far side of the road link: left as it is
            why['far_side'] += 1
            continue
        try:
            new = pl.offset_curve(kerb * (wp / 2 + clearance_m + ws / 2))
        except Exception:
            why['offset_failed'] += 1
            continue
        if new.is_empty or new.geom_type != "LineString" or new.distance(mid) > 12.0:
            why['offset_far'] += 1
            continue
        # move the footpath's own lane onto the frame where it runs alongside the road, vertex by vertex (it keeps its length,
        # its ends and its way on past the road): beyond the stretch the road covers the move fades out over 10 m
        import numpy as np
        pts = np.array(shapely.segmentize(cl, 2.0).coords)
        a0, a1, b0, b1 = (np.array(new.coords[k]) for k in (0, 1, -2, -1))
        t0, t1 = (a1 - a0) / (np.linalg.norm(a1 - a0) or 1.0), (b1 - b0) / (np.linalg.norm(b1 - b0) or 1.0)
        moved = []
        for v in pts:
            t = new.interpolate(new.project(shapely.Point(v)))
            over = max(0.0, -float(np.dot(v - a0, t0)), float(np.dot(v - b1, t1)))      # metres past either end of the stretch
            w = max(0.0, 1.0 - over / 10.0)
            moved.append((v[0] + w * (t.x - v[0]), v[1] + w * (t.y - v[1])))
        seg = LineString(moved)
        if seg.length < 0.5:
            why["cut_short"] += 1
            continue
        out.append((LineString([(x / (kx * M) + x0, y / M + y0) for x, y in seg.coords]).wkt, lane_id))
    if out:
        con.executemany(f"UPDATE {sch}.lane SET geom = ST_GeomFromText(?::VARCHAR) WHERE lane_id = ?::VARCHAR", out)
    logger.info(f"GMNS[{sch[5:]}]: {len(out):,} footpath lanes placed from their road's frame; left as they were: "
                + (", ".join(f"{n:,} {k.replace('_', ' ')}" for k, n in why.most_common()) or "none"))


def _walk_kerb(con, sch, clearance_m=0.0):
    """A mapped sidewalk (``footway=sidewalk``) that still lies on a road after ``_walk_frame`` (its road was on its far side, too far, or none
    was found) is pushed out to the kerb: every vertex that lies within a driving lane running ALONG the sidewalk moves to that road's edge plus
    half the sidewalk's width (+ ``clearance_m``), so one side of a street is not lost. A vertex on a lane that crosses the sidewalk (a side
    road's mouth) is left alone: that is a crossing, not a sidewalk. Only mapped sidewalks move; none is added. Lane geometry only."""
    import numpy as np
    import shapely
    from shapely import wkt as _w
    from shapely.geometry import LineString, Point
    from shapely.ops import nearest_points

    if not con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name = 'gmns_driving' AND table_name = 'lane'").fetchone()[0]:
        return
    kids = con.execute(f"""SELECT l.lane_id, ST_AsText(l.geom), l.width FROM {sch}.lane l JOIN {sch}.link k ON k.link_id = l.link_id
                           WHERE k.footway = 'sidewalk' AND l.geom IS NOT NULL AND l.allowed_uses = 'walk'""").fetchall()
    roads = con.execute(f"""SELECT ST_AsText(geom), COALESCE(width, {_DEFAULT_LANE_W}) FROM gmns_driving.lane
                            WHERE geom IS NOT NULL AND allowed_uses IN ('auto', 'bus')""").fetchall()
    if not kids or not roads:
        return
    x0, y0 = _w.loads(roads[0][0]).coords[0]
    kx, M = (math.cos(math.radians(y0)) or 1.0), 111320.0
    to_m = lambda g: LineString([((x - x0) * kx * M, (y - y0) * M) for x, y in g.coords])        # noqa: E731
    lanes = [to_m(_w.loads(w)) for w, _ in roads]
    polys = [ln.buffer(wd / 2, cap_style="flat") for ln, (_, wd) in zip(lanes, roads)]
    tree = shapely.STRtree(polys)
    out = []
    for lane_id, wkt, wd in kids:
        half = (wd or _DEFAULT_WALK_W) / 2 + clearance_m
        ln = shapely.segmentize(to_m(_w.loads(wkt)), 2.0)
        pts = np.array(ln.coords)
        if len(pts) < 2:
            continue
        tang = np.gradient(pts, axis=0)
        tang /= np.maximum(np.hypot(tang[:, 0], tang[:, 1]), 1e-9)[:, None]
        moved, changed = pts.copy(), False
        for i, v in enumerate(pts):
            p = Point(v)
            near = []
            for j in tree.query(p.buffer(25.0)):
                lane = lanes[j]
                s0 = lane.project(p)
                a, b = lane.interpolate(max(s0 - 0.5, 0)), lane.interpolate(min(s0 + 0.5, lane.length))
                n = math.hypot(b.x - a.x, b.y - a.y) or 1.0
                if abs(((b.x - a.x) * tang[i][0] + (b.y - a.y) * tang[i][1]) / n) >= 0.7:   # a lane that runs along the sidewalk
                    near.append(polys[j])
            if not near:
                continue
            nx, ny = -tang[i][1], tang[i][0]                              # across the sidewalk: across the road
            cut = LineString([(v[0] - nx * 25.0, v[1] - ny * 25.0), (v[0] + nx * 25.0, v[1] + ny * 25.0)]).intersection(
                shapely.union_all(near).buffer(0.05))                     # the road along that line (lanes that touch are one road)
            ts = []                                                       # the road's stretches along the line, as (from, to) distances from v
            for g in getattr(cut, "geoms", [cut]):
                if g.geom_type == "LineString" and not g.is_empty:
                    u = [(x - v[0]) * nx + (y - v[1]) * ny for x, y in g.coords]
                    ts.append((min(u) + 0.05, max(u) - 0.05))                   # the 0.05 m the road was grown by, taken back
            here = [t for t in ts if t[0] - half <= 0 <= t[1] + half]     # the stretch the vertex is on, or within the clearance of
            if not here:
                continue
            t0, t1 = min(here, key=lambda t: min(abs(t[0]), abs(t[1])) if not t[0] <= 0 <= t[1] else 0)
            if t0 <= 0 <= t1:                                             # inside the road: out through the nearer kerb
                shift = t1 + half if t1 <= -t0 else t0 - half
            elif t0 > 0:                                                  # outside, the road ahead on the + side, closer than the clearance
                shift = t0 - half
            else:                                                         # outside, the road behind
                shift = t1 + half
            if abs(shift) > 1e-6:
                moved[i] = v + np.array([nx, ny]) * shift
                changed = True
        if changed:
            for _ in range(2):                                             # a light smoothing: no ragged edge where the push changes
                moved[1:-1] = (moved[:-2] + 2 * moved[1:-1] + moved[2:]) / 4
            out.append((LineString([(x / (kx * M) + x0, y / M + y0) for x, y in moved]).wkt, lane_id))
    if out:
        con.executemany(f"UPDATE {sch}.lane SET geom = ST_GeomFromText(?::VARCHAR) WHERE lane_id = ?::VARCHAR", out)
    logger.info(f"GMNS[{sch[5:]}]: {len(out):,} sidewalk lanes pushed out of a road, to its kerb")


def _walk_join(con, sch):
    """Footways that meet at a node end at that one point in OSM, and must after ``_walk_frame`` and ``_walk_kerb`` moved a sidewalk's end: the
    sidewalk was moved, the crosswalk or steps at the same node were not, and the joint opened up (Monaco: over half of the pairs of lane ends
    at a node more than 0.5 m apart). Every moved sidewalk end at a node gives the node a new place (their mean); every footway, path, steps
    or crosswalk lane that ends at the node then ends there. Only lane geometry, and only footway classes: a road walked on is untouched."""
    import numpy as np
    from shapely import wkt as _w
    from shapely.geometry import LineString

    nonroad = ", ".join(repr(h) for h in _NON_MOTOR)
    rows = con.execute(f"""SELECT l.lane_id, k.from_node_id, k.to_node_id, k.footway, ST_AsText(l.geom)
                           FROM {sch}.lane l JOIN {sch}.link k ON k.link_id = l.link_id
                           WHERE l.geom IS NOT NULL AND l.allowed_uses = 'walk'
                             AND regexp_replace(split_part(k.facility_type, ';', 1), '_link$', '') IN ({nonroad})""").fetchall()
    nodes = {r[0]: (r[1], r[2]) for r in con.execute(f"SELECT node_id, x_coord, y_coord FROM {sch}.node").fetchall()}
    if not rows or not nodes:
        return
    x0, y0 = next(iter(nodes.values()))
    kx, M = (math.cos(math.radians(y0)) or 1.0), 111320.0
    lanes, ends = {}, {}
    for lane_id, fn, tn, fw, wkt in rows:
        pts = np.array([((x - x0) * kx * M, (y - y0) * M) for x, y in _w.loads(wkt).coords])
        if len(pts) < 2 or fn not in nodes or tn not in nodes:
            continue
        lanes[lane_id] = pts
        ends.setdefault(fn, []).append((lane_id, 0, fw))
        ends.setdefault(tn, []).append((lane_id, -1, fw))
    changed = set()
    for nd, lst in ends.items():
        if len(lst) < 2:
            continue
        home = np.array([(nodes[nd][0] - x0) * kx * M, (nodes[nd][1] - y0) * M])
        moved = [lanes[ln][i] for ln, i, fw in lst if fw == "sidewalk" and np.hypot(*(lanes[ln][i] - home)) > 0.05]
        if not moved:
            continue
        target = np.mean(moved, axis=0)
        for ln, i, fw in lst:
            if np.hypot(*(lanes[ln][i] - target)) > 0.02:
                lanes[ln][i] = target
                changed.add(ln)
    out = [(LineString([(x / (kx * M) + x0, y / M + y0) for x, y in lanes[ln]]).wkt, ln) for ln in changed]
    if out:
        con.executemany(f"UPDATE {sch}.lane SET geom = ST_GeomFromText(?::VARCHAR) WHERE lane_id = ?::VARCHAR", out)
    logger.info(f"GMNS[{sch[5:]}]: {len(out):,} footway lanes joined at the nodes their sidewalks were moved from")


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
             CASE WHEN t.type = 'uturn' THEN NULL   -- the spec's code has no U: `type` says it
                  ELSE (CASE WHEN t.hdg < 45 OR t.hdg >= 315 THEN 'NB' WHEN t.hdg < 135 THEN 'EB'
                             WHEN t.hdg < 225 THEN 'SB' ELSE 'WB' END)
                       || (CASE t.type WHEN 'left' THEN 'L' WHEN 'right' THEN 'R' ELSE 'T' END)
             END AS mvmt_code,
             '{uses}' AS allowed_uses,
             ST_AsText({_bezier_line_sql('t.x0', 't.y0', 't.cx0', 't.cy0',
                                         't.cx1', 't.cy1', 't.x1', 't.y1')}) AS geometry,
             t.ang AS _ang
      FROM cp t
      LEFT JOIN _sig sig ON sig.osm_id = t.node_id""")
    _assign_lanes(con, sch, mode, drive_side)
    _continuation_movements(con, sch)


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


# road classes, high to low: at a fork the exit of the highest class goes on as the main road
_CLASS_RANK = ["motorway", "trunk", "primary", "secondary", "tertiary", "unclassified", "residential",
               "living_street", "service"]


def _fork_lanes(n, obs, main, angs=None):
    """Lanes where a road goes on (Kaveh, 2026-09-30, option B; forks and junctions): the exit that
    goes on as the road (index ``main`` in ``obs``, sorted left to right) keeps all its lanes, lane by
    lane, on the side away from the others; every other exit shares the inbound lanes on its own side
    (left of the main exit: the leftmost lanes, right of it: the rightmost) - as many as it has at a
    fork, one at a junction (a turn of 45 degrees or more, ``angs``), as osm2gmns gives a turn. Per
    exit a pair of 0-based ranges of equal length."""
    out, m = [], obs[main]
    c = min(n, m)
    left_only = main > 0 and main == len(obs) - 1
    for k, mb in enumerate(obs):
        if k == main:
            out.append(((n - c, n - 1), (m - c, m - 1)) if left_only else ((0, c - 1), (0, c - 1)))
        else:
            b = 1 if angs is not None and abs(angs[k]) >= 45 else min(mb, n)
            out.append(((0, b - 1), (0, b - 1)) if k < main else ((n - b, n - 1), (mb - b, mb - 1)))
    return out


def _assign_lanes(con, sch, mode, drive_side="right"):
    """Fill each movement's inbound / outbound lane ranges (docs/design/gmns_lane_movements.md):

    - ``turn:lanes`` only on the last piece of its OSM way, at the junction where the way ends (the
      arrows apply "to the junction", OSM wiki Key:turn): each exit takes the lanes whose arrow
      matches its place among the exits (``_turn_side``);
    - along a way (a movement into the next piece of the same way, same direction) every lane
      continues lane by lane, so marked lanes run on to their junction;
    - into a merge (a node with one outbound link), osm2gmns's merge rule (``_merge_lanes``);
    - at a fork, the road that goes on keeps all its lanes and a branch shares its side (``_fork_lanes``);
    - else osm2gmns's junction rule (``_default_lanes``).

    Ranges have equal length and pair in order: the k-th inbound lane into the k-th outbound lane.
    Then the GMNS types ``diverge`` / ``merge`` (``_fork_types``). Drops the helper column ``_ang``."""
    from collections import defaultdict

    import pandas as pd

    nl = dict(con.execute(f"""SELECT l.link_id, max(l.lane_num) FROM {sch}.lane l      -- the motor lanes: not the
        LEFT JOIN _extra_lane x ON x.link_id = l.link_id AND x.lane_num = l.lane_num   -- bike lane beside them
        WHERE x.link_id IS NULL GROUP BY 1""").fetchall())
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
    name_of, cls_of = {}, {}
    for lk, nm, ft in con.execute(f"SELECT link_id, name, facility_type FROM {sch}.link").fetchall():
        name_of[lk], cls_of[lk] = nm, ft
    n_in = defaultdict(int)
    for nd in ends.values():
        n_in[nd] += 1
    rank = lambda lk: _CLASS_RANK.index(str(cls_of.get(lk) or "").replace("_link", "")) \
        if str(cls_of.get(lk) or "").replace("_link", "") in _CLASS_RANK else len(_CLASS_RANK)
    rows = []
    for ib, ms in by_ib.items():
        n = nl.get(ib, 1)
        ms.sort(key=lambda m: uturn_side if m[2] == "uturn" else m[3], reverse=True)   # left to right
        obs = [nl.get(m[1], 1) for m in ms]
        way_ends = not any(along(ib, m[1]) for m in ms)  # the way ends here: its arrows apply
        default = _default_lanes(n, obs)
        real = [k for k, m in enumerate(ms) if m[2] != "uturn"]
        ahead = [k for k in real if abs(ms[k][3]) < 45]
        if len(real) >= 2 and ahead:
            # a road that goes on (fork or junction): the exit ahead that continues it (its name, its OSM
            # way, the higher class, the straightest) keeps its lanes, the others share their side;
            # U-turns keep osm2gmns's lane
            main = min(ahead, key=lambda k: (name_of.get(ms[k][1]) is None or name_of.get(ms[k][1]) != name_of.get(ib),
                                             not along(ib, ms[k][1]), rank(ms[k][1]), abs(ms[k][3])))
            for k, rng in zip(real, _fork_lanes(n, [obs[x] for x in real], real.index(main), [ms[x][3] for x in real])):
                default[k] = rng
        elif len(real) == 1 and abs(ms[real[0]][3]) >= 45:
            # a single turn enters from its own side: a right turn into the rightmost lanes (osm2gmns
            # fills from the left, so a right turn went into lane 1)
            k = real[0]
            c = min(n, obs[k])
            default[k] = ((n - c, n - 1), (obs[k] - c, obs[k] - 1)) if ms[k][3] < 0 else ((0, c - 1), (0, c - 1))
        sides = _turn_side(ms)
        extra = defaultdict(set)         # arrows no exit matches go to the nearest exit on their side,
        if way_ends and ib in kinds:     # else to the exit ahead (Avenue de Fontvieille: left|left|)
            here = set().union(*kinds[ib].values())
            thru = next((k for k in real if sides[k] == "thru"), None)
            for kind in ("left", "right"):
                if kind in here and kind not in {sides[k] for k in real}:
                    side_ks = [k for k in real if thru is None or (k < thru if kind == "left" else k > thru)]
                    target = (side_ks[0] if kind == "left" else side_ks[-1]) if side_ks else thru
                    if target is not None:
                        extra[target].add(kind)
        for k, ((mid, ob, typ, _), mo, rng, side) in enumerate(zip(ms, obs, default, sides)):
            want = {side} | extra.get(k, set())
            tagged = sorted(num for num, ks in kinds.get(ib, {}).items() if ks & want) if way_ends else []
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


def _continuation_movements(con, sch):
    """Movement rows for the lanes no movement leaves (Kaveh, 2026-10-02: ``167625718#2f`` lanes 2 and 3 had "no way out"). Where a link has exactly one way on
    (U-turns aside), a car lane its movement does not cover (the road has fewer lanes ahead) merges into the last car lane of the next link (``merge``), and a bike
    lane goes on into the next link's bike lane (``thru``, ``allowed_uses`` bike): one row each, ``<mvmt_id>-<lane>``, in the same node and link pair. The lane connectors
    and every reader follow from the rows."""
    lanes = con.execute(f"SELECT link_id, lane_num, allowed_uses, turn FROM {sch}.lane").fetchall()
    car, bike, marked = defaultdict(list), defaultdict(list), set()
    for lk, n, use, turn in lanes:
        (bike if use == "bike" else car)[lk].append(n)
        if turn:
            marked.add(lk)                           # OSM says what each lane is for (turn:lanes): a lane no exit takes is meant to end there
    outs = defaultdict(list)
    for mid, ib, ob, si, ei in con.execute(f"SELECT mvmt_id, ib_link_id, ob_link_id, start_ib_lane, end_ib_lane FROM {sch}.movement "
                                           f"WHERE type <> 'uturn'").fetchall():
        outs[ib].append((mid, ob, si, ei))
    new = []
    for ib, ms in outs.items():
        if len(ms) != 1 or ib not in car or ib in marked:
            continue
        mid, ob, si, ei = ms[0]
        if not car.get(ob):
            continue
        covered = set(range(si or 1, (ei or si or 1) + 1))
        for n in sorted(car[ib]):
            if n not in covered:
                new.append((mid, f"{mid}-{n}", n, n, max(car[ob]), max(car[ob]), "merge", None))
        if bike.get(ib) and bike.get(ob):
            new.append((mid, f"{mid}-b{bike[ib][0]}", bike[ib][0], bike[ib][0], bike[ob][0], bike[ob][0], "thru", "bike"))
    if not new:
        return
    import pandas as pd
    _cont = pd.DataFrame(new, columns=["base", "new_id", "si", "ei", "so", "eo", "typ", "uses"])   # noqa: F841 (read by SQL)
    con.execute(f"""INSERT INTO {sch}.movement SELECT m.* REPLACE (c.new_id AS mvmt_id, c.si AS start_ib_lane, c.ei AS end_ib_lane,
                      c.so AS start_ob_lane, c.eo AS end_ob_lane, c.typ AS type, COALESCE(c.uses, m.allowed_uses) AS allowed_uses)
                    FROM {sch}.movement m JOIN _cont c ON c.base = m.mvmt_id""")
    logger.info(f"GMNS[{sch[5:]}]: {len(new):,} movements for lanes that go on where the road has fewer lanes or a bike lane continues")


def _fork_types(con, sch, max_ang=45.0):
    """The GMNS movement types ``diverge`` and ``merge`` (docs/design/gmns_lane_movements.md):
    ``diverge`` at a fork, a node one link arrives at, its 2+ ways on (U-turns aside) all within
    ``max_ang`` degrees of straight on; ``merge`` into a node one link leaves, its 2+ inbound links
    all joining within ``max_ang``. The angle bound keeps an ordinary junction (a one-way street
    meeting a cross street) a junction. ``mvmt_code`` keeps the angle's letter (GMNS allows R/L/T; none for a U-turn)."""
    con.execute(f"""UPDATE {sch}.movement m SET type = 'diverge' WHERE type <> 'uturn' AND m.node_id IN (
        SELECT f.node_id FROM {sch}.movement f
        WHERE (SELECT count(*) FROM {sch}.link k WHERE k.to_node_id = f.node_id) = 1 AND f.type <> 'uturn'
        GROUP BY f.node_id HAVING count(*) >= 2 AND max(abs(f._ang)) < {max_ang})""")
    con.execute(f"""UPDATE {sch}.movement m SET type = 'merge' WHERE type <> 'uturn' AND m.ob_link_id IN (
        SELECT f.ob_link_id FROM {sch}.movement f
        WHERE (SELECT count(*) FROM {sch}.link k WHERE k.from_node_id = f.node_id) = 1 AND f.type <> 'uturn'
        GROUP BY f.ob_link_id HAVING count(*) >= 2 AND max(abs(f._ang)) < {max_ang})""")


def _fit_width(pts, w, floor=0.6):
    """``w`` narrowed where the curve ``pts`` is tighter than the lane: at most twice the tightest radius (a turn
    of 5 m through 90 degrees can't carry a 3.25 m lane), never under ``floor`` of ``w`` (a neck between two lanes)."""
    rmin = float("inf")
    for p, q, r in zip(pts, pts[1:], pts[2:]):
        a, b, c = math.dist(p, q), math.dist(q, r), math.dist(p, r)
        area2 = abs((q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]))
        if area2 > 1e-9:
            rmin = min(rmin, a * b * c / (2 * area2))      # circumradius = abc / 4 area
    return max(floor * w, min(w, 1.8 * rmin))


def _inherit_lanes(edges, passes=6):
    """Lane counts for one-way roads OSM gives none for (Kaveh, 2026-10-02: the tunnel ``80378487`` after the 2-lane ``167625718``).
    ``edges``: dicts ``edge_id``, ``source``, ``target``, ``cls`` (the road class), ``lanes`` (as the edges table has it: tagged, else a
    default of 1), ``tagged`` (the way has a ``lanes`` / ``turn:lanes`` / ``width:lanes`` tag), ``oneway``. An untagged one-way edge takes the
    lanes of the road it plainly continues (the node between them joins only the two, both one-way, of one class: a road changes its name
    at a tunnel) when that has more; else of the one it plainly leads into. Passes repeat, so a way cut into pieces inherits piece by piece.
    Returns ``{edge_id: lanes}`` for the edges that change."""
    by = {e["edge_id"]: dict(e) for e in edges}
    at = defaultdict(set)                                # node -> the edges that touch it
    for e in by.values():
        at[e["source"]].add(e["edge_id"])
        at[e["target"]].add(e["edge_id"])
    changed = {}
    for _ in range(passes):
        step = False
        for e in by.values():
            if e["tagged"] or not e["oneway"]:
                continue
            for node, end in ((e["source"], "target"), (e["target"], "source")):      # what leads in, else what it leads into
                others = at[node] - {e["edge_id"]}
                if len(others) != 1:
                    continue
                o = by[next(iter(others))]
                if o["oneway"] and o["cls"] == e["cls"] and o[end] == node and (o["lanes"] or 1) > (e["lanes"] or 1):
                    e["lanes"] = changed[e["edge_id"]] = o["lanes"]
                    step = True
                    break
        if not step:
            break
    return changed


def _infer_lanes(con, mode, has_raw):
    """``_lanes_inf(edge_id, lanes)`` (a temp table): :func:`_inherit_lanes` over the mode's edges. Empty without the raw OSM tags (no way to
    tell a tagged ``lanes`` from the default)."""
    con.execute("CREATE OR REPLACE TEMP TABLE _lanes_inf(edge_id BIGINT, lanes INTEGER)")
    if not has_raw or mode != "driving":
        return
    cols = {c for (c,) in con.execute("SELECT column_name FROM duckdb_columns() WHERE database_name = 's' AND schema_name = ? "
                                      "AND table_name = 'edges'", [mode]).fetchall()}
    if "oneway" not in cols:
        return
    rows = con.execute(f"""SELECT e.edge_id, e.source, e.target, e.name, regexp_replace(split_part(e.highway, ';', 1), '_link$', ''), e.lanes,
        (w.tags['lanes'] IS NOT NULL OR w.tags['turn:lanes'] IS NOT NULL OR w.tags['width:lanes'] IS NOT NULL OR w.tags['lanes:forward'] IS NOT NULL),
        e.oneway FROM s.{mode}.edges e LEFT JOIN s.raw.ways w ON w.osm_id = abs(e.osm_id)""").fetchall()
    edges = [dict(edge_id=a, source=b, target=c, cls=f, lanes=g, tagged=bool(h), oneway=bool(i)) for a, b, c, d, f, g, h, i in rows]
    got = _inherit_lanes(edges)
    if got:
        con.executemany("INSERT INTO _lanes_inf VALUES (?, ?)", list(got.items()))
        logger.info(f"GMNS[{mode}]: {len(got):,} one-way edges without a lanes tag take the lane count of the road they continue")


def _build_lane_connectors(con, sch, gap_ok=0.25, max_trim=0.4, junction_pad=0.5, min_keep_m=2.0):
    """Lanes that connect (docs/design/gmns_lane_connectors.md): shorten each lane where it ends
    inside a junction (the other links' lanes, but its own link's reverse) or doesn't meet the lane
    it leads into, then join every lane pair of a movement with a cubic Bézier along both lanes, in
    ``lane_connector``. Works in one local metre frame for the area."""
    import shapely
    from shapely import wkt as _w
    from shapely.geometry import LineString, Point
    from shapely.ops import substring

    lanes = con.execute(f"SELECT lane_id, link_id, lane_num, COALESCE(width, CASE WHEN allowed_uses = 'walk' THEN {_DEFAULT_WALK_W} WHEN allowed_uses = 'bike' THEN {_DEFAULT_BIKE_W} ELSE {_DEFAULT_LANE_W} END), ST_AsText(geom) "
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
    runs = {}                                    # link -> (run, position), from _build_lane_curb
    try:
        runs = {lk: (r, k, n, c) for lk, r, k, n, c in con.execute(
            "SELECT link_id, run_id, seq, n, closed FROM _lane_runs").fetchall()}
    except Exception:                            # noqa: BLE001 - no run table (lanes without geometry)
        pass
    num_of = {lid: num for lid, _, num, _, _ in lanes}

    def continues(a, b):
        """Lane b is lane a going on into the next piece of the same run: one road, not cut, no
        connector. Only when the two really meet (the run's offset curve may have fallen back to
        per-piece offsets): otherwise the pair is trimmed and connected like any other."""
        ra, rb = runs.get(link_of[a]), runs.get(link_of[b])
        if ra is None or rb is None or ra[0] != rb[0] or num_of[a] != num_of[b]:
            return False
        if not (rb[1] == ra[1] + 1 or (ra[3] and ra[1] == ra[2] - 1 and rb[1] == 0)):   # a ring wraps
            return False
        return Point(geom[a].coords[-1]).distance(Point(geom[b].coords[0])) <= 0.3

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

    going_on = {(a, b) for _, a, b in pairs if continues(a, b)}
    keep_end = {a for a, _ in going_on}          # these ends stay where they are: the road goes on
    keep_start = {b for _, b in going_on}
    # a node a road goes on through: a lane joining or leaving there isn't cut back from the road's
    # surface either (cut along its centre line it left a triangle of background at its far corner);
    # it runs to the node and overlaps the through road, which is drawn in the same colour
    through = {ends[link_of[a]][1] for a, _ in going_on}
    trim = {lid: [0.0, 0.0] for lid in geom}     # [at the start, at the end] in metres
    for lid, line in geom.items():
        a, b = ends[link_of[lid]]
        trim[lid] = [0.0 if lid in keep_start or a in through else inside_len(line, a, link_of[lid], False),
                     0.0 if lid in keep_end or b in through else inside_len(line, b, link_of[lid], True)]
    for _, a, b in pairs:                        # room for an S-curve where the lanes don't meet
        if (a, b) in going_on:
            continue
        gap = Point(geom[a].coords[-1]).distance(Point(geom[b].coords[0]))
        if gap > gap_ok:                         # an end that goes on stays: the turn's curve leaves from it
            need = max(3.0, 2.5 * gap) / 2
            if a not in keep_end:
                trim[a][1] = max(trim[a][1], need)
            if b not in keep_start:
                trim[b][0] = max(trim[b][0], need)
    cut = {}
    for lid, line in geom.items():
        n = line.length
        s0, s1 = (min(t, n * max_trim) for t in trim[lid])
        room = n - max(min_keep_m, 0.5 * n)      # trimming never leaves less than 2 m / half the lane
        if s0 + s1 > room > 0:
            s0, s1 = s0 * room / (s0 + s1), s1 * room / (s0 + s1)
        elif room <= 0:
            s0 = s1 = 0.0
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
        if (a, b) in going_on:                   # one road going on: nothing to connect
            continue
        p0, p3 = cut[a].coords[-1], cut[b].coords[0]
        chord = math.dist(p0, p3)
        if chord < 0.3:
            continue
        (ax, ay), (bx, by) = tangent(cut[a], True), tangent(cut[b], False)
        p1, p2 = (p0[0] + ax * 0.4 * chord, p0[1] + ay * 0.4 * chord), (p3[0] - bx * 0.4 * chord, p3[1] - by * 0.4 * chord)
        pts = [tuple((1 - t) ** 3 * u + 3 * (1 - t) ** 2 * t * v + 3 * (1 - t) * t ** 2 * w + t ** 3 * z
                     for u, v, w, z in zip(p0, p1, p2, p3)) for t in (i / 12 for i in range(13))]
        rows.append((f"{a}>{b}", mid, a, b, _fit_width(pts, min(width[a], width[b])), to_ll(pts)))
    import pandas as pd
    _cut = pd.DataFrame([(lid, to_ll(line.coords)) for lid, line in cut.items() if line is not geom[lid]],  # noqa: F841
                        columns=["lane_id", "wkt"])
    if len(_cut):
        con.execute(f"UPDATE {sch}.lane l SET geom = ST_GeomFromText(c.wkt) FROM _cut c WHERE c.lane_id = l.lane_id")
    if rows:
        _con = pd.DataFrame(rows, columns=["connector_id", "mvmt_id", "from_lane_id", "to_lane_id", "width", "wkt"])  # noqa: F841
        con.execute(f"INSERT INTO {sch}.lane_connector SELECT connector_id, mvmt_id, from_lane_id, to_lane_id, width, "
                    f"ST_GeomFromText(wkt) FROM _con")


# OSM point features that GMNS's `location` table holds (the standard recommends OSM names for loc_type)
_LOC_HIGHWAY = ("crossing", "bus_stop", "give_way", "stop", "traffic_signals", "toll_gantry",
                "mini_roundabout", "speed_camera")


def _build_location(con, sch, mode, has_raw, create_empty=False):
    """``location``: the OSM nodes that lie on a link and carry one of the tags above, one row per link they
    lie on. ``lr`` is the distance in metres from the link's from-node, along its shape. Needs the raw tags
    (no ``raw`` schema, no table). docs/exports/gmns_tables.md."""
    if not has_raw:
        if create_empty:               # the transit stops of a GTFS feed still go here
            con.execute(f"""CREATE TABLE {sch}.location(loc_id VARCHAR, link_id BIGINT, ref_node_id BIGINT, lr DOUBLE,
              x_coord DOUBLE, y_coord DOUBLE, z_coord DOUBLE, loc_type VARCHAR, zone_id BIGINT, gtfs_stop_id VARCHAR,
              osm_id BIGINT, name VARCHAR, geom GEOMETRY)""")
        return
    hw = ", ".join(repr(h) for h in _LOC_HIGHWAY)
    ecols = _src_cols(con, mode, "edges")
    # height: between the link's two end heights, by distance along it (not on a bridge or in a tunnel)
    elev = {"z_from", "z_to"} <= ecols
    z = "e.z_from + (e.z_to - e.z_from) * lr / NULLIF(link_len, 0)" if elev else "NULL"
    struct = [f"COALESCE(e.{c}, 'no') NOT IN ('no', '')" for c in ("tunnel", "bridge") if c in ecols]
    zskip = " OR ".join(struct) if elev and struct else "false"
    need = ", ".join(["edge_id"] + [c for c in ("z_from", "z_to", "tunnel", "bridge") if c in ecols])
    ej = f"LEFT JOIN (SELECT {need} FROM s.{mode}.edges) e ON e.edge_id = pos.link_id" if elev else ""
    con.execute(f"""CREATE TABLE {sch}.location AS
      WITH pts AS (
        SELECT n.osm_id, n.lon, n.lat, ST_Point(n.lon, n.lat) AS p, n.tags['name'] AS name,
               CASE WHEN n.tags['highway'] IN ({hw}) THEN n.tags['highway']
                    WHEN n.tags['traffic_calming'] IS NOT NULL THEN 'traffic_calming'
                    WHEN n.tags['amenity'] = 'parking_entrance' THEN 'parking_entrance'
                    WHEN n.tags['railway'] = 'level_crossing' THEN 'level_crossing' END AS loc_type
        FROM s.raw.nodes n
        WHERE n.tags['highway'] IN ({hw}) OR n.tags['traffic_calming'] IS NOT NULL
           OR n.tags['amenity'] = 'parking_entrance' OR n.tags['railway'] = 'level_crossing'
      ), hit AS (
        SELECT pts.*, k.link_id, k.from_node_id, k.length AS link_len, k.geom AS lg,
               ST_LineLocatePoint(k.geom, pts.p) AS f
        FROM pts JOIN {sch}.link k ON ST_DWithin(k.geom, pts.p, 2e-6)   -- ~0.2 m: on the link's shape
      )
      , pos AS (
        SELECT hit.*, CASE WHEN f <= 0 THEN 0.0 WHEN f >= 1 THEN link_len
                           ELSE ST_Length_Spheroid(ST_FlipCoordinates(ST_LineSubstring(lg, 0, f))) END AS lr
        FROM hit)
      SELECT osm_id::VARCHAR || '_' || link_id::VARCHAR AS loc_id, link_id, from_node_id AS ref_node_id, lr,
             lon AS x_coord, lat AS y_coord,
             CASE WHEN {zskip} THEN NULL ELSE ({z})::DOUBLE END AS z_coord, loc_type, 1::BIGINT AS zone_id,
             NULL::VARCHAR AS gtfs_stop_id, osm_id, name, p AS geom
      FROM pos {ej} ORDER BY link_id, lr""")


def _add_transit_stops(con, sch, mode, stops, drive_side, max_m=30.0):
    """The GTFS stops as ``location`` rows with their ``gtfs_stop_id`` (the standard's link to GTFS).

    Each stop goes on the nearest link of this mode within ``max_m`` metres: in the driving schema the stops a bus
    calls at, in the walking and cycling schemas every stop (people walk to a tram or train stop too). Of a
    two-way road's two links it takes the one the stop is on the kerb side of (right with right-hand traffic).
    ``lr`` is the distance along the link; the coordinates are the stop's own. An OpenStreetMap ``bus_stop``
    within 25 m of a GTFS stop is the same stop, so its row is replaced by the GTFS one."""
    import numpy as np
    import shapely
    from shapely import wkt as _w
    from shapely.geometry import Point
    from shapely.strtree import STRtree

    todo = stops[stops.is_bus] if mode == "driving" else stops
    links = con.execute(f"SELECT link_id, from_node_id, length, ST_AsText(geom) FROM {sch}.link").fetchall()
    if len(todo) == 0 or not links:
        return
    scale = np.array([math.cos(math.radians(float(todo.lat.iloc[0]))) * 111320.0, 111320.0])
    geoms = [shapely.transform(_w.loads(l[3]), lambda c: c * scale) for l in links]
    tree = STRtree(geoms)
    kerb = -1.0 if drive_side == "right" else 1.0
    rows, far = [], 0
    for st in todo.itertuples():
        p = Point(st.lon * scale[0], st.lat * scale[1])
        best = None
        for i in tree.query(p, predicate="dwithin", distance=max_m):
            g = geoms[i]
            t = g.project(p)
            a, b = g.interpolate(max(t - 1.0, 0.0)), g.interpolate(min(t + 1.0, g.length))
            q = g.interpolate(t)
            side = (b.x - a.x) * (p.y - q.y) - (b.y - a.y) * (p.x - q.x)             # > 0: left of the link
            key = (round(g.distance(p) * 2) / 2, 0 if side * kerb > 0 else 1)         # nearest, then kerb side
            if best is None or key < best[0]:
                best = (key, i, g.project(p, normalized=True))
        if best is None:
            far += 1
            continue
        link_id, from_node, length, _ = links[best[1]]
        rows.append((f"gtfs_{st.stop_id}_{link_id}", link_id, from_node, best[2] * length, st.lon, st.lat,
                     st.loc_type, st.stop_id, st.name))
    if rows:
        import pandas as pd
        df = pd.DataFrame(rows, columns=["loc_id", "link_id", "ref_node_id", "lr", "x_coord", "y_coord", "loc_type",
                                         "gtfs_stop_id", "name"])
        con.register("_gtfs_loc", df)
        con.execute(f"""INSERT INTO {sch}.location SELECT loc_id, link_id, ref_node_id, lr, x_coord, y_coord,
            NULL::DOUBLE, loc_type, 1::BIGINT, gtfs_stop_id, NULL::BIGINT, name, ST_Point(x_coord, y_coord)
            FROM _gtfs_loc""")
        con.unregister("_gtfs_loc")
        con.execute(f"""DELETE FROM {sch}.location o WHERE o.gtfs_stop_id IS NULL AND o.loc_type = 'bus_stop' AND EXISTS (
            SELECT 1 FROM {sch}.location g WHERE g.gtfs_stop_id IS NOT NULL AND g.loc_type = 'bus_stop'
              AND sqrt(pow((o.x_coord - g.x_coord) * cos(radians(o.y_coord)) * 111320, 2)
                       + pow((o.y_coord - g.y_coord) * 111320, 2)) < 25)""")
    logger.info(f"GTFS[{mode}]: {len(rows)} of {len(todo)} stops on a link"
                + (f", {far} farther than {max_m:g} m from any" if far else ""))


def _apply_signs(con, sch, mode, has_raw, max_before_m=50.0):
    """``ctrl_type`` ``stop`` / ``yield`` from OSM ``highway=stop`` / ``give_way`` signs (the spec's category
    list: yield, stop, stop_2_way, stop_4_way, signal). A sign lies on a way node on the approach, so it is
    the inbound link's sign when it is within ``max_before_m`` of the link's end, before the junction, and
    its ``direction`` (forward / backward / both, the way's direction) matches the link's. Signs without a
    direction, or exactly on the junction node, say nothing about which approach and are left out. Every
    movement from such a link, unless its node is signalised, gets the control; the node gets ``stop_4_way``
    (every approach stops, four or more), ``stop`` (every approach, fewer), ``stop_2_way`` (some) or ``yield``.
    Cars and bikes only: pedestrians do not stop for a stop sign."""
    if not has_raw or mode == "walking" or not _exists(con, "s", mode, "edges"):
        return
    con.execute(f"""CREATE OR REPLACE TEMP TABLE _sign AS
      SELECT x.link_id AS ib, bool_or(x.loc_type = 'stop') AS stop
      FROM {sch}.location x JOIN {sch}.link k ON k.link_id = x.link_id
        JOIN s.raw.nodes n ON n.osm_id = x.osm_id JOIN s.{mode}.edges e ON e.edge_id = x.link_id
      WHERE x.loc_type IN ('stop', 'give_way') AND x.lr >= k.length - {max_before_m} AND x.lr < k.length - 0.5
        AND ((n.tags['direction'] IN ('forward', 'both') AND NOT e.is_reverse)
          OR (n.tags['direction'] IN ('backward', 'both') AND e.is_reverse))
      GROUP BY 1""")
    con.execute(f"""UPDATE {sch}.movement m SET ctrl_type = CASE WHEN g.stop THEN 'stop' ELSE 'yield' END
      FROM _sign g WHERE m.ib_link_id = g.ib AND m.ctrl_type IS NULL""")
    con.execute(f"""UPDATE {sch}.node SET ctrl_type = c.t FROM (
        SELECT k.to_node_id AS node_id, count(*) AS n_in, count(g.ib) AS n_sign, count(*) FILTER (WHERE g.stop) AS n_stop,
               CASE WHEN count(*) FILTER (WHERE g.stop) = count(*) AND count(*) >= 4 THEN 'stop_4_way'
                    WHEN count(*) FILTER (WHERE g.stop) = count(*) THEN 'stop'
                    WHEN count(*) FILTER (WHERE g.stop) > 0 THEN 'stop_2_way' ELSE 'yield' END AS t
        FROM {sch}.link k LEFT JOIN _sign g ON g.ib = k.link_id GROUP BY 1 HAVING count(g.ib) > 0) c
      WHERE node.node_id = c.node_id AND node.ctrl_type IS NULL""")


def _node_types(con, sch, has_raw):
    """``node_type``: OSM-style names, as the spec's FAQ recommends: the OSM ``highway`` value for a
    turning circle, mini roundabout or motorway junction; else ``intersection`` where three or more
    neighbours meet, ``dead_end`` where one does. Other nodes (where a road is merely cut) stay empty."""
    osm = ("CASE WHEN n.tags['highway'] IN ('turning_circle', 'mini_roundabout', 'motorway_junction') "
           "THEN n.tags['highway'] END") if has_raw else "NULL"
    join = "LEFT JOIN s.raw.nodes n ON n.osm_id = nd.node_id" if has_raw else ""
    con.execute(f"""UPDATE {sch}.node SET node_type = c.t FROM (
        WITH nb AS (SELECT node_id, count(DISTINCT other) AS deg FROM (
                      SELECT from_node_id AS node_id, to_node_id AS other FROM {sch}.link
                      UNION ALL SELECT to_node_id, from_node_id FROM {sch}.link) GROUP BY 1)
        SELECT nd.node_id, COALESCE({osm}, CASE WHEN nb.deg >= 3 THEN 'intersection' WHEN nb.deg = 1 THEN 'dead_end' END) AS t
        FROM {sch}.node nd LEFT JOIN nb USING (node_id) {join}) c
      WHERE node.node_id = c.node_id AND c.t IS NOT NULL""")


def _build_zone(con, sch, area):
    """``zone``: the one zone every node is in (``node.zone_id`` = 1). Its outline is the area's boundary when
    the source db was built with one; without, only the name (the output file's). GMNS consumers such as
    Path4GMNS need at least one zone on the nodes."""
    cols = _exists(con, "s", "main", "boundary") and {c for (c,) in con.execute(
        "SELECT column_name FROM duckdb_columns() WHERE database_name = 's' AND schema_name = 'main' "
        "AND table_name = 'boundary'").fetchall()}
    if cols and "geom" in cols and con.execute("SELECT count(*) FROM s.main.boundary").fetchone()[0] > 0:
        name = "string_agg(DISTINCT name, ', ')" if "name" in cols else "NULL::VARCHAR"
        con.execute(f"""CREATE TABLE {sch}.zone AS SELECT 1::BIGINT AS zone_id, coalesce({name}, ?) AS name,
          ST_AsText(ST_Union_Agg(geom)) AS boundary, NULL::VARCHAR AS super_zone, ST_Union_Agg(geom) AS geom
          FROM s.main.boundary""", [area])
    else:
        con.execute(f"""CREATE TABLE {sch}.zone AS SELECT 1::BIGINT AS zone_id, ? AS name,
          NULL::VARCHAR AS boundary, NULL::VARCHAR AS super_zone, NULL::GEOMETRY AS geom""", [area])


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

    raw_join = "LEFT JOIN s.raw.ways w ON w.osm_id = abs(e.osm_id)" if has_raw else ""   # a virtual (negative) id is its OSM way with a minus: its tags are the way's
    tagcol = "w.tags" if has_raw else "NULL::MAP(VARCHAR, VARCHAR)"
    has_oneway = con.execute(
        "SELECT count(*) FROM duckdb_columns() WHERE database_name = 's' AND schema_name = ? "
        "AND table_name = 'edges' AND column_name = 'oneway'", [mode]).fetchone()[0] > 0
    oneway_sel = "e.oneway" if has_oneway else "false"      # older/synthetic edges: treat as two-way
    rows = con.execute(f"""SELECT e.edge_id, e.is_reverse, COALESCE(li.lanes, e.lanes), e.length_m, e.source,
      {oneway_sel} AS oneway, ST_AsText(e.geometry) AS wkt, {tagcol} AS tags, e.target, e.osm_id, e.name
      FROM s.{mode}.edges e {raw_join} LEFT JOIN _lanes_inf li ON li.edge_id = e.edge_id""").fetchall()
    side_sign = -1.0 if drive_side == "right" else 1.0  # offset_curve(+) is left; right-hand → negative

    def pick(tags, base, is_rev):
        # OSM: unsuffixed *:lanes is the way's forward direction; :backward is the reverse edge
        return tags.get(base + ":backward") if is_rev else (
            tags.get(base + ":forward") or tags.get(base))

    def bike_extra(tags, is_rev):
        """An on-road bike lane (``cycleway=lane``) on the RIGHT of this edge's direction of travel, as
        ``(drawn width, tagged width or None)``: the way's right side for its forward edge, its left side for
        the reverse edge. Right-hand traffic only: with left-hand traffic the lane is on the left and would
        renumber the motor lanes. None where a ``bicycle:lanes`` lane already is the bike lane."""
        if (mode != "driving" or drive_side != "right"
                or "designated" in (_split(pick(tags, "bicycle:lanes", is_rev)) or [])):
            return None
        for key in (f"cycleway:{'left' if is_rev else 'right'}", "cycleway:both", "cycleway"):
            if tags.get(key) == "lane":
                w = _num(tags.get(f"{key}:width")) or _num(tags.get("cycleway:width"))
                return (w or _DEFAULT_BIKE_LANE_W, w)
        return None

    def is_foot(tags):
        """A footpath of the walking network: 2 m wide and one strip for both directions (people walk both ways on it, so
        its two links are not two sides of a road): centred on its line like a one-way road, never paired."""
        return mode == "walking" and tags.get("highway") in _NON_MOTOR

    def lane_widths(tags, lanes, is_rev):
        turns = _split(pick(tags, "turn:lanes", is_rev))
        widths = _split(pick(tags, "width:lanes", is_rev))
        # one lane per `turn:lanes` entry, and never fewer than `lanes`: an OSM way tagged lanes=3 with only two
        # turn entries has a third lane that the arrows say nothing about (a pocket can add lanes, never remove)
        n = max(len(turns), int(lanes or 1), 1)
        extra = bike_extra(tags, is_rev)
        dw = _DEFAULT_WALK_W if is_foot(tags) else _default_lane_w(tags.get("highway"))
        return turns, widths, [(_num(widths[i]) if i < len(widths) else None) or dw
                               for i in range(n)] + ([extra[0]] if extra else [])

    # runs of pieces (docs/design/gmns_lane_runs.md): one road is a chain of edges of one OSM way;
    # its lanes are placed once for the whole run (pairing, placement, the offset curve)
    def headings(wkt):
        """The way's heading at its start and at its end, degrees, from its first / last 2 points."""
        try:
            pts = [tuple(map(float, p.split())) for p in wkt[wkt.index("(") + 1:wkt.rindex(")")].split(",")]
        except (ValueError, AttributeError):
            return 0.0, 0.0
        if len(pts) < 2:
            return 0.0, 0.0
        k = math.cos(math.radians(pts[0][1]))
        h = lambda p, q: math.degrees(math.atan2(q[1] - p[1], (q[0] - p[0]) * k))  # noqa: E731
        return h(pts[0], pts[1]), h(pts[-2], pts[-1])

    info, w_of, place_of = {}, {}, {}
    for edge_id, is_rev, lanes, length_m, source, oneway, wkt, tags, target, osm_id, name in rows:
        tags = tags or {}
        w_each = lane_widths(tags, lanes, is_rev)[2]
        w_of[edge_id] = w_each
        place_of[edge_id] = _placement(pick(tags, "placement", is_rev), w_each) if (oneway or is_foot(tags) or side_sign < 0) else None
        h0, h1 = headings(wkt) if wkt else (0.0, 0.0)
        info[edge_id] = (osm_id, bool(is_rev), source, target, bool(oneway) or is_foot(tags), tuple(w_each), name,
                         tags.get("junction") == "roundabout", h0, h1)
    rows_tags = {r[0]: r[7] or {} for r in rows}
    runs, where, closed = _chain_runs(info)
    wkt_of = {r[0]: r[6] for r in rows}
    merged = {}                                  # run index -> the run's line, lon/lat wkt
    for i, run in enumerate(runs):
        cs = []
        for e in run:
            if not wkt_of[e]:
                break
            w = wkt_of[e]
            c = [p.strip() for p in w[w.index("(") + 1:w.rindex(")")].split(",")]   # "LINESTRING (x y, ...)"
            cs += c[1:] if cs and c[0] == cs[-1] else c
        else:
            merged[i] = "LINESTRING (" + ", ".join(cs) + ")"
    gaps, run_profiles = {}, {}
    if lane_geometry and pair_carriageways:
        run_gaps = _paired_gaps([(i, merged[i], sum(w_of[run[0]]) / 2.0) for i, run in enumerate(runs)
                                 if i in merged and info[run[0]][4] and not is_foot(rows_tags[run[0]])],
                                side_sign, profiles=run_profiles)
        gaps = {e: run_gaps[i] for i, run in enumerate(runs) if i in run_gaps for e in run}
        if gaps:
            logger.info(f"GMNS[{mode}]: {len(gaps):,} one-way edges placed with their opposite carriageway")
    run_place = {i: next((place_of[e] for e in run if place_of[e] is not None), None) for i, run in enumerate(runs)}

    def offsets(edge_id):
        """Each lane's offset from the road line (left +), the rule per edge as before."""
        w_each, oneway = w_of[edge_id], info[edge_id][4]
        total = sum(w_each)
        half, gap, place, out, run = total / 2.0, gaps.get(edge_id), run_place[where[edge_id][0]], [], 0.0
        for w in w_each:
            # distance of this lane's centre from the line its direction's lanes start at (the centre line):
            # lane 1 is always the LEFTMOST lane (OSM's turn:lanes, the movements and the GMNS convention
            # count from the left). With right-hand traffic the lanes lie right of that line, so the leftmost
            # is nearest it; with left-hand traffic they lie left of it, so the leftmost is the farthest.
            d = run + w / 2.0 if side_sign < 0 else total - run - w / 2.0
            if place is not None:                            # OSM placement: from where the line lies
                out.append(place - (run + w / 2.0))
            elif oneway and gap is not None:                 # one side of a road mapped as 2 ways
                out.append(side_sign * (d - gap / 2.0))
            elif oneway:                                     # one-way: lanes centered on the carriageway
                out.append(half - (run + w / 2.0))
            else:                                            # two-way: this direction's lanes on its travel side
                out.append(side_sign * d)
            run += w
        return out

    run_geoms = {}                               # edge -> [lane wkt per lane], from the run's one curve
    if lane_geometry:
        for i, run in enumerate(runs):
            if len(run) > 1 and i in merged:
                per_piece = _run_lane_wkts([wkt_of[e] for e in run], offsets(run[0]), closed=i in closed)
                if per_piece:
                    for e, lane_wkts in zip(run, per_piece, strict=True):
                        run_geoms[e] = lane_wkts
        import pandas as pd
        con.register("_runs_df", pd.DataFrame([(e, i, k, len(runs[i]), i in closed) for e, (i, k) in where.items()],
                                              columns=["link_id", "run_id", "seq", "n", "closed"]))
        con.execute("CREATE OR REPLACE TEMP TABLE _lane_runs AS SELECT * FROM _runs_df")
        con.unregister("_runs_df")

    lane_rows, curb_rows, extra_lanes = [], [], []
    for edge_id, is_rev, lanes, length_m, source, oneway, wkt, tags, target, osm_id, _name in rows:
        tags = tags or {}
        turns, widths, w_each = lane_widths(tags, lanes, is_rev)
        bikes = _split(pick(tags, "bicycle:lanes", is_rev))
        psvs = _split(pick(tags, "psv:lanes", is_rev))
        buses = _split(pick(tags, "bus:lanes", is_rev))     # bus lanes are tagged either way
        n = len(w_each)
        extra = bike_extra(tags, is_rev)                     # the last lane, beside the motor lanes
        offs = offsets(edge_id)
        run = 0.0
        for i in range(n):
            u = uses
            if extra and i == n - 1:
                u = "bike"
                extra_lanes.append((edge_id, i + 1))
            elif i < len(bikes) and bikes[i] in ("designated", "yes"):
                u = "bike"
            elif (i < len(psvs) and psvs[i] in ("designated", "yes")) or \
                    (i < len(buses) and buses[i] in ("designated", "yes")):
                u = "bus"
            width = extra[1] if extra and i == n - 1 else (_num(widths[i]) if i < len(widths) else None)
            if width is None and u not in ("bike", "walk"):
                width = w_each[i]                            # the class default, so every reader draws the same width
            turn = turns[i] if i < len(turns) and turns[i] not in ("", "none") else None
            run += w_each[i]
            if not lane_geometry:
                geom = None
            elif edge_id in run_geoms:                       # from the run's one offset curve
                geom = run_geoms[edge_id][i]
            else:
                geom = _offset_wkt(wkt, offs[i])
            ri = where[edge_id][0]
            if geom and edge_id in gaps and ri in run_profiles and run_place[ri] is None:   # the gap varies along the road
                geom = _vary_wkt(geom, merged[ri], run_profiles[ri], gaps[edge_id], side_sign)
            lane_rows.append((f"{edge_id}_{i + 1}", edge_id, i + 1, u, None, None, width, turn, geom))
        for side in ("left", "right", "both"):
            val = tags.get(f"parking:{side}") or tags.get(f"parking:lane:{side}")
            if val and val not in ("no", "separate", "none"):
                curb_rows.append((f"{edge_id}_{side}", edge_id, source, 0.0, length_m, val, None))

    con.execute("CREATE TEMP TABLE IF NOT EXISTS _extra_lane(link_id BIGINT, lane_num INTEGER)")
    if extra_lanes:       # bike lanes beside the motor lanes: movements, forks and merges ignore them
        con.executemany("INSERT INTO _extra_lane VALUES (?, ?)", extra_lanes)
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
                            GREATEST(1, CEIL({_len_m('geom')} / {cl}))::BIGINT AS nc
                     FROM (SELECT lane_id, link_id, lane_num, ST_RemoveRepeatedPoints(geom) AS geom
                           FROM {g}.lane WHERE geom IS NOT NULL) WHERE ST_NPoints(geom) >= 2)
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
                            GREATEST(1, CEIL({_len_m('la.geom')} / {cl}))::BIGINT AS nc
                     FROM (SELECT * REPLACE (ST_RemoveRepeatedPoints(geom) AS geom) FROM {g}.lane
                           WHERE geom IS NOT NULL) la JOIN {g}.link lk ON lk.link_id = la.link_id
                     WHERE ST_NPoints(la.geom) >= 2)
          SELECT 'C' || lane_id || '#' || k AS link_id, lane_id || '@' || k AS from_node_id,
                 lane_id || '@' || (k + 1) AS to_node_id, 1 AS dir_flag,
                 {_len_m('ST_LineSubstring(geom, k::DOUBLE/nc, LEAST(1.0,(k+1.0)/nc))')}::DOUBLE AS length,
                 1 AS lanes, width, free_speed, facility_type, allowed_uses,
                 ST_AsText(ST_LineSubstring(geom, k::DOUBLE/nc, LEAST(1.0,(k+1.0)/nc))) AS geometry,
                 ST_LineSubstring(geom, k::DOUBLE/nc, LEAST(1.0,(k+1.0)/nc)) AS geom,
                 link_id AS macro_link_id, 'M' || link_id AS meso_link_id, lane_num AS lane_no,
                 'normal' AS cell_type, NULL::VARCHAR AS mvmt_txt_id, NULL::VARCHAR AS ctrl_type
          FROM L, range(nc) AS s(k)""")
        # lane-change micro links: adjacent lanes of a link, one cell forward (both directions)
        con.execute(f"""INSERT INTO {mc}.micro_link ({_MICRO_COLS})
          SELECT 'H' || a.node_id || '-' || b.node_id, a.node_id, b.node_id, 1,
                 {_len_m('ST_MakeLine(a.geom, b.geom)')}, 1, NULL::DOUBLE, NULL::DOUBLE, 'lane_change',
                 NULL::VARCHAR, ST_AsText(ST_MakeLine(a.geom, b.geom)), ST_MakeLine(a.geom, b.geom),
                 a.macro_link_id, 'M' || a.macro_link_id, NULL::INTEGER, 'lane_change',
                 NULL::VARCHAR, NULL::VARCHAR
          FROM {mc}.micro_node a JOIN {mc}.micro_node b
            ON a.macro_link_id = b.macro_link_id AND abs(a.lane_no - b.lane_no) = 1
               AND b.cell_k = a.cell_k + 1""")
        # movement (turn) micro links: inbound lane end -> outbound lane start. The path is the lane_connector's
        # curve for that lane pair where there is one (lanes stop short of a junction, and it joins their ends);
        # otherwise the lanes already meet and a straight line joins the two cell nodes
        has_lc = con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name = ? AND table_name = "
                             "'lane_connector'", [g]).fetchone()[0] > 0
        path = "COALESCE(lc.geom, ST_MakeLine(ie.pt, oe.pt))" if has_lc else "ST_MakeLine(ie.pt, oe.pt)"
        lc_join = (f"LEFT JOIN {g}.lane_connector lc ON lc.mvmt_id = m.mvmt_id AND lc.from_lane_id = ie.lane_id "
                   f"AND lc.to_lane_id = oe.lane_id") if has_lc else ""
        con.execute(f"""INSERT INTO {mc}.micro_link ({_MICRO_COLS})
          WITH ie AS (SELECT macro_link_id AS lk, lane_no, arg_max(node_id, cell_k) AS node,
                             arg_max(geom, cell_k) AS pt, arg_max(lane_id, cell_k) AS lane_id
                      FROM {mc}.micro_node GROUP BY macro_link_id, lane_no),
               oe AS (SELECT macro_link_id AS lk, lane_no, arg_min(node_id, cell_k) AS node,
                             arg_min(geom, cell_k) AS pt, arg_min(lane_id, cell_k) AS lane_id
                      FROM {mc}.micro_node GROUP BY macro_link_id, lane_no),
               t AS (SELECT m.*, il.free_speed, ie.node AS inode, oe.node AS onode, {path} AS path
                     FROM {g}.movement m      -- one connector per lane pair of the movement's ranges, in order
                     JOIN ie ON ie.lk = m.ib_link_id
                       AND ie.lane_no BETWEEN COALESCE(m.start_ib_lane, 1) AND COALESCE(m.end_ib_lane, m.start_ib_lane, 1)
                     JOIN oe ON oe.lk = m.ob_link_id
                       AND oe.lane_no = COALESCE(m.start_ob_lane, 1) + ie.lane_no - COALESCE(m.start_ib_lane, 1)
                     JOIN {g}.link il ON il.link_id = m.ib_link_id
                     {lc_join})
          SELECT 'X' || inode || '-' || onode, inode, onode, 1, {_len_m('path')}, 1, NULL::DOUBLE, free_speed,
                 'connector', allowed_uses, ST_AsText(path), path,
                 NULL::BIGINT, 'M' || ib_link_id, NULL::INTEGER, 'movement', mvmt_code, ctrl_type
          FROM t""")
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
             1 AS dir_flag, {_len_m('ST_LineSubstring(geom, tf, 1 - tf)')}::DOUBLE AS length, lanes::INTEGER AS lanes,
             NULL::DOUBLE AS capacity, free_speed, facility_type, allowed_uses,
             ST_AsText(ST_LineSubstring(geom, tf, 1 - tf)) AS geometry,
             ST_LineSubstring(geom, tf, 1 - tf) AS geom,
             'normal' AS meso_type, link_id AS macro_link_id, NULL::VARCHAR AS movement_id,
             NULL::VARCHAR AS mvmt_txt_id, NULL::INTEGER AS start_ib_lane, NULL::INTEGER AS end_ib_lane,
             NULL::VARCHAR AS ctrl_type
      FROM L""")

    # connectors: a smooth curve from the end of the inbound section to the start of the outbound one (the
    # sections are trimmed at both ends, so the macro movement's curve would start and end short of them)
    k = 111320.0
    con.execute(f"""INSERT INTO {ms}.meso_link
      (link_id, from_node_id, to_node_id, dir_flag, length, lanes, capacity, free_speed,
       facility_type, allowed_uses, geometry, geom, meso_type, macro_link_id, movement_id,
       mvmt_txt_id, start_ib_lane, end_ib_lane, ctrl_type)
      WITH c1 AS (
        SELECT m.*, il.free_speed AS fs,
               ST_X(ST_EndPoint(ib.geom)) AS x0, ST_Y(ST_EndPoint(ib.geom)) AS y0,
               ST_X(ST_StartPoint(ob.geom)) AS x1, ST_Y(ST_StartPoint(ob.geom)) AS y1,
               ST_X(ST_PointN(ib.geom, (ST_NPoints(ib.geom) - 1)::INTEGER)) AS px,
               ST_Y(ST_PointN(ib.geom, (ST_NPoints(ib.geom) - 1)::INTEGER)) AS py,
               ST_X(ST_PointN(ob.geom, 2)) AS nx, ST_Y(ST_PointN(ob.geom, 2)) AS ny
        FROM {g}.movement m
        JOIN {ms}.meso_link ib ON ib.link_id = 'M' || m.ib_link_id::VARCHAR AND ib.meso_type = 'normal'
        JOIN {ms}.meso_link ob ON ob.link_id = 'M' || m.ob_link_id::VARCHAR AND ob.meso_type = 'normal'
        JOIN {g}.link il ON il.link_id = m.ib_link_id
      ), c2 AS (   -- unit directions in metres (x scaled by cos latitude), and the chord's length
        SELECT *, cos(radians(y0)) AS kx,
               (x0 - px) * cos(radians(y0)) AS dx0, (y0 - py) AS dy0,
               (nx - x1) * cos(radians(y1)) AS dx1, (ny - y1) AS dy1,
               sqrt(pow((x1 - x0) * cos(radians(y0)) * {k}, 2) + pow((y1 - y0) * {k}, 2)) AS chord
        FROM c1
      ), cp AS (   -- control points 40 % of the chord out along each end's direction
        SELECT *, x0 + dx0 / GREATEST(sqrt(dx0*dx0 + dy0*dy0), 1e-12) * 0.4 * chord / ({k} * kx) AS cx0,
                  y0 + dy0 / GREATEST(sqrt(dx0*dx0 + dy0*dy0), 1e-12) * 0.4 * chord / {k} AS cy0,
                  x1 - dx1 / GREATEST(sqrt(dx1*dx1 + dy1*dy1), 1e-12) * 0.4 * chord / ({k} * kx) AS cx1,
                  y1 - dy1 / GREATEST(sqrt(dx1*dx1 + dy1*dy1), 1e-12) * 0.4 * chord / {k} AS cy1
        FROM c2
      ), t AS (
        SELECT *, {_bezier_line_sql('x0', 'y0', 'cx0', 'cy0', 'cx1', 'cy1', 'x1', 'y1')} AS path FROM cp
      )
      SELECT 'X' || ib_link_id::VARCHAR || '-' || ob_link_id::VARCHAR,
             ib_link_id::VARCHAR || 'd', ob_link_id::VARCHAR || 'u', 1,
             {_len_m('path')}::DOUBLE,
             COALESCE(end_ib_lane - start_ib_lane + 1, 1)::INTEGER, NULL::DOUBLE, fs,
             'connector', allowed_uses,
             ST_AsText(path), path,
             'movement', NULL::BIGINT, mvmt_id,
             CASE type WHEN 'left' THEN 'L' WHEN 'right' THEN 'R' WHEN 'uturn' THEN 'U'
                       ELSE 'T' END,
             start_ib_lane, end_ib_lane, ctrl_type          -- the movement's own lanes
      FROM t""")


def _dump_csv(con, modes, to_csv, schema_prefix="gmns_", csv_extensions=False):
    """COPY each GMNS table to a spec-standard CSV (dropping the non-spec geom/turn columns).
    Skips tables a schema doesn't have (e.g. the combined ``gmns_all`` carries only node/link/config/
    use_*)."""
    tables = ["config", "node", "link", "geometry", "lane", "movement",
              "use_definition", "use_group", "signal_controller", "curb_seg", "location", "zone"]
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
            if excl and csv_extensions:        # keep the extension columns, as the spec's user-defined `u_` fields
                kinds = dict(con.execute("SELECT column_name, data_type FROM duckdb_columns() WHERE "
                                         "schema_name = ? AND table_name = ?", [sch, t]).fetchall())
                sel += "".join(f", {c} AS u_{c}" for c in excl if kinds.get(c) != "GEOMETRY")
            path = os.path.join(d, f"{t}.csv")
            con.execute(f"COPY (SELECT {sel} FROM {sch}.{t}) TO '{path}' (HEADER, DELIMITER ',')")
