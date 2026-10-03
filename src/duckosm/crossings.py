"""GMNS crossings: where a zebra is painted, on which lanes, on which stretch of each (docs/design/gmns_crossings.md).

Two tables in ``gmns_driving``, duckOSM extensions like ``lane_connector``: ``crossing`` (one row per crossing) and
``lane_crossing`` (one row per crossing per driving lane it covers: the stretch of that lane, and where the lane lies across the
zebra, so a reader paints the stripes lane by lane and they stay continuous from lane to lane).
"""
import math

_ROAD_USES = ("auto", "bus")                       # a zebra is painted on driving lanes; a bike lane is no part of it
_DEFAULT_LANE_W = 3.25
_ZEBRA_W_PER_ROAD_W = 0.4                           # the zebra's width, from the width of the road it crosses ...
_ZEBRA_W_MIN, _ZEBRA_W_MAX = 2.5, 4.0               # ... between these, metres (an OSM `width` tag wins)
_MIN_ANGLE_SIN = 0.5                                # a crossing lies across the road: 30 degrees or more from the lane
_MAX_WIND = 1.3                                     # length / chord of a piece
_ROAD_ANGLE = 30.0                                  # degrees: covered lanes this close in direction are one road (one rectangle)
_MIN_ALONG = math.cos(math.radians(_ROAD_ANGLE))     # |cos| of a lane's direction to the rectangle's axis: a lane of another road is not under it ...
_MIN_ALONG_CONNECTOR = 0.5                          # ... a connector curves through the junction: 60 degrees
_MAX_OVERLAP = 0.10                                 # of a crossing's footprint on one already placed: the same crossing
_NODE_REACH_M = 25.0                                # a node crossing's line, to each side of the node


def _level(layer, bridge, tunnel):
    """The level of a way or link: its OSM ``layer`` if that is a number, else 1 for a bridge, -1 for a tunnel, else 0 (the rule roadstyle
    and lanestyle use to put a lane in a band). A crossing paints only on the lanes of its own level."""
    try:
        ly = int(float(layer))
    except (TypeError, ValueError):
        ly = 0
    if ly:
        return ly
    yes = lambda v: v is not None and str(v) not in ("", "no")          # noqa: E731
    return 1 if yes(bridge) else -1 if yes(tunnel) else 0


def _painted(crossing, markings):
    """Are stripes painted (the OSM tags say a marked crossing)? ``crossing=unmarked`` / ``crossing:markings=no`` no; a crossing with
    markings, or a marked / zebra / uncontrolled one, yes; signals alone or no tag at all, no."""
    if crossing == "unmarked" or markings == "no":
        return False
    return markings is not None or crossing in ("marked", "zebra", "uncontrolled")


def _interp(x, xs, ys):
    """``ys`` at ``x`` along the increasing ``xs`` (the end value outside)."""
    if x <= xs[0]:
        return ys[0]
    for i in range(1, len(xs)):
        if x <= xs[i]:
            t = (x - xs[i - 1]) / ((xs[i] - xs[i - 1]) or 1.0)
            return ys[i - 1] + t * (ys[i] - ys[i - 1])
    return ys[-1]


def _num(v, default):
    try:
        w = float(str(v).replace(",", "."))
        return w if 0.5 <= w <= 20 else default
    except (TypeError, ValueError):
        return default


def _zebra_width(tagged, road_width):
    """The zebra's width along the road: the OSM ``width`` if tagged, else estimated from the width of the road it crosses (the span
    across the covered lanes): ``_ZEBRA_W_PER_ROAD_W`` of it, between ``_ZEBRA_W_MIN`` and ``_ZEBRA_W_MAX``."""
    return tagged or min(max(_ZEBRA_W_PER_ROAD_W * road_width, _ZEBRA_W_MIN), _ZEBRA_W_MAX)


def build_crossings(con, sch, has_raw):
    """Create ``{sch}.crossing`` and ``{sch}.lane_crossing`` (empty without the raw OSM tags or without lane geometry)."""
    con.execute(f"""CREATE TABLE {sch}.crossing(crossing_id VARCHAR, source VARCHAR, crossing_type VARCHAR, markings VARCHAR,
                    painted BOOLEAN, width DOUBLE, length DOUBLE, osm_id BIGINT, geom GEOMETRY)""")
    con.execute(f"""CREATE TABLE {sch}.lane_crossing(crossing_id VARCHAR, lane_id VARCHAR, link_id BIGINT, start_lr DOUBLE,
                    end_lr DOUBLE, across_from DOUBLE, across_to DOUBLE, to_left BOOLEAN)""")
    if not has_raw:
        return
    import shapely
    from shapely import wkt as _w
    from shapely.geometry import LineString, Point, Polygon

    lanes = con.execute(f"""SELECT l.lane_id, l.link_id, COALESCE(l.width, {_DEFAULT_LANE_W}), ST_AsText(l.geom), k.osm_id,
                                   k.layer, k.bridge, k.tunnel, false
                            FROM {sch}.lane l JOIN {sch}.link k ON k.link_id = l.link_id
                            WHERE l.geom IS NOT NULL AND l.allowed_uses IN {_ROAD_USES}""").fetchall()
    if con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name = ? AND table_name = 'lane_connector'", [sch]).fetchone()[0]:
        # the lane connectors are road surface too: duckOSM trims a lane where a junction starts and joins it to the next by a connector, so
        # a zebra over a joint is on the lanes and on the connector between them
        lanes += con.execute(f"""SELECT c.connector_id, l.link_id, COALESCE(c.width, {_DEFAULT_LANE_W}), ST_AsText(c.geom), k.osm_id,
                                        k.layer, k.bridge, k.tunnel, true
                                 FROM {sch}.lane_connector c JOIN {sch}.lane l ON l.lane_id = c.from_lane_id
                                 JOIN {sch}.link k ON k.link_id = l.link_id
                                 WHERE c.geom IS NOT NULL AND l.allowed_uses IN {_ROAD_USES}""").fetchall()
    ways = con.execute("""SELECT osm_id, tags['crossing'], tags['crossing:markings'], tags['width'], refs, tags['layer'], tags['bridge'],
                                 tags['tunnel'] FROM s.raw.ways
                          WHERE tags['footway'] = 'crossing'""").fetchall()
    nodes = con.execute("""SELECT osm_id, lon, lat, tags['crossing'], tags['crossing:markings'] FROM s.raw.nodes
                           WHERE tags['highway'] = 'crossing'""").fetchall()
    if not lanes or not (ways or nodes):
        return
    x0, y0 = _w.loads(lanes[0][3]).coords[0]
    kx, M = (math.cos(math.radians(y0)) or 1.0), 111320.0
    to_m = lambda lon, lat: ((lon - x0) * kx * M, (lat - y0) * M)                    # noqa: E731
    to_ll = lambda x, y: (x / (kx * M) + x0, y / M + y0)                             # noqa: E731

    lane_id, link_id, lane_way, lane_w, lane_level, lane_conn = [], [], [], [], [], []
    geom, poly = [], []
    for lid, lk, w, wkt, osm, ly, br, tu, is_conn in lanes:
        g = LineString([to_m(*p) for p in _w.loads(wkt).coords])
        if g.length < 0.5:
            continue
        lane_id.append(lid)
        link_id.append(lk)
        lane_way.append(osm)
        lane_level.append(_level(ly, br, tu))
        lane_conn.append(is_conn)
        lane_w.append(w)
        geom.append(g)
        poly.append(g.buffer(w / 2, cap_style="flat"))
    tree = shapely.STRtree(poly)

    def tangent(lane, p):
        s = lane.project(p)
        a, b = lane.interpolate(max(s - 0.5, 0)), lane.interpolate(min(s + 0.5, lane.length))
        n = math.hypot(b.x - a.x, b.y - a.y) or 1.0
        return (b.x - a.x) / n, (b.y - a.y) / n

    def pieces(line, only=None, level=None):
        """The parts of ``line`` that lie across the road: its stretches inside the driving lanes' surface, straight enough and at
        30 degrees or more from the lane they lie on. ``only``: lane indexes it may use."""
        near = [j for j in tree.query(line.buffer(0.1)) if (only is None or j in only) and (level is None or lane_level[j] == level)]
        if not near:
            return [], []
        on = line.intersection(shapely.union_all([poly[j] for j in near]).buffer(0.1))      # lanes that touch are one road
        on = shapely.line_merge(on) if on.geom_type == "MultiLineString" else on
        out = []
        for part in getattr(on, "geoms", [on]):
            if part.geom_type != "LineString" or part.length < 2.0:
                continue
            a, b = part.coords[0], part.coords[-1]
            chord = math.hypot(b[0] - a[0], b[1] - a[1])
            if chord < 1.0 or part.length > _MAX_WIND * chord:
                continue
            mid = part.interpolate(0.5, normalized=True)
            lane = min(near, key=lambda j: geom[j].distance(mid))
            tx, ty = tangent(geom[lane], mid)
            if abs((b[0] - a[0]) * ty - (b[1] - a[1]) * tx) / chord < _MIN_ANGLE_SIN:
                continue
            out.append(part)
        return out, near

    # every crossing: (source, osm_id, type, markings, width, the line, the lanes it may use)
    cands = []
    node_xy = {}
    for osm, ct, mk, wd, refs, ly, br, tu in ways:
        cands.append(["w", osm, ct, mk, _num(wd, None), refs, None, _level(ly, br, tu)])
    ref_ids = {r for c in cands for r in c[5]}
    if ref_ids:
        for osm, lon, lat in con.execute("SELECT osm_id, lon, lat FROM s.raw.nodes WHERE list_contains(?, osm_id)", [list(ref_ids)]).fetchall():
            node_xy[osm] = to_m(lon, lat)
    done = []
    for c in cands:
        pts = [node_xy[r] for r in c[5] if r in node_xy]
        if len(pts) >= 2:
            c[5] = LineString(pts)
            done.append(c)
    cands = done
    on_crossing_way = {r for w in ways for r in w[4]}
    road_ways = {}
    for osm, refs in con.execute(f"SELECT osm_id, refs FROM s.raw.ways WHERE osm_id IN (SELECT DISTINCT osm_id FROM {sch}.link)").fetchall():
        for r in refs:
            road_ways.setdefault(r, set()).add(osm)
    for osm, lon, lat, ct, mk in nodes:
        if osm in on_crossing_way or len(road_ways.get(osm, ())) != 1:        # a crossing way has it, or a junction (not a mid-road crossing)
            continue
        way = next(iter(road_ways[osm]))
        only = {j for j in range(len(geom)) if lane_way[j] == way}
        p = Point(to_m(lon, lat))
        near = [j for j in only if geom[j].distance(p) < 12.0]
        if not near:
            continue
        lane = min(near, key=lambda j: geom[j].distance(p))
        tx, ty = tangent(geom[lane], p)
        line = LineString([(p.x - ty * _NODE_REACH_M, p.y + tx * _NODE_REACH_M), (p.x + ty * _NODE_REACH_M, p.y - tx * _NODE_REACH_M)])
        cands.append(["n", osm, ct, mk, None, line, only, None])

    found = []                                  # (length, candidate, part, lane indexes)
    for c in cands:
        parts, near = pieces(c[5], None, c[7])        # a node crossing's line also runs across the carriageways of the neighbouring ways (a dual road)
        if c[0] == "n" and parts:               # the part through the node itself, not another stretch of the same line
            parts = [min(parts, key=lambda q: q.distance(Point(c[5].interpolate(0.5, normalized=True))))]
        if parts:                               # all the pieces of one crossing (the road's carriageways, a median between) are one crossing
            found.append((sum(q.length for q in parts), c, parts, near))
    crossing_rows, lane_rows, placed, seen = [], [], None, {}
    for _, c, parts, near in sorted(found, key=lambda f: -f[0]):             # the long crossings first: they win a junction
        src, osm, ct, mk = c[0], c[1], c[2], c[3]
        # 1. the lanes the crossing goes through, and where along each piece of it (inside the road)
        cov = []                                # (lane index, piece, s0, s1)
        for part in parts:
            for j in near:
                inter = part.intersection(poly[j])
                if inter.is_empty or getattr(inter, "length", 0) < 0.3:
                    continue
                ss = [part.project(Point(pt)) for g in getattr(inter, "geoms", [inter]) for pt in getattr(g, "coords", [])]
                if max(ss) - min(ss) >= 0.3:
                    cov.append((j, part, min(ss), max(ss)))
        if not cov:
            continue
        if c[0] == "n" and not any(j in c[6] for j, _, _, _ in cov):      # a node crossing is on ITS road: the way the node lies on must be crossed
            continue
        # At a junction the crossing line also touches the lanes of the other road. The zebra is across the road the crossing goes *across*:
        # keep the lanes most nearly perpendicular to the crossing line (within 0.15 in sine of the best), so the other road's lanes
        # do not turn the zebra's direction (an average of two roads points at neither)
        def _sin(item):
            j, part, s0, s1 = item
            a, b = part.interpolate(s0), part.interpolate(s1)
            n = math.hypot(b.x - a.x, b.y - a.y) or 1.0
            tx, ty = tangent(geom[j], part.interpolate((s0 + s1) / 2))
            return abs(((b.x - a.x) * ty - (b.y - a.y) * tx) / n)
        sins = [_sin(it) for it in cov]
        cov = [it for it, sn in zip(cov, sins) if sn >= max(sins) - 0.15]
        # 2. the roads it crosses: the covered lanes grouped by direction (axes within _ROAD_ANGLE degrees, opposite ones are one axis: a dual
        #    carriageway is one road). A crossing over a junction corner is two roads, each with its own rectangle: an average of two roads'
        #    directions points at neither (docs/design/gmns_crossings.md, "One rectangle per road")
        roads = []                              # [anchor angle, [(lane index, piece, s0, s1, tangent)]]
        for j, part, s0, s1 in cov:
            t = tangent(geom[j], part.interpolate((s0 + s1) / 2))
            ang = math.degrees(math.atan2(t[1], t[0])) % 180
            for road in roads:
                if abs((ang - road[0] + 90) % 180 - 90) < _ROAD_ANGLE:
                    road[1].append((j, part, s0, s1, t))
                    break
            else:
                roads.append([ang, [(j, part, s0, s1, t)]])
        if c[0] == "n":                         # a crossing node is on ITS road: another road's lanes the node's line also touches are no zebra of it
            roads = [r for r in roads if any(j in c[6] for j, *_ in r[1])] or roads
        fresh = []                              # the rectangles of this crossing: they may overlap each other at a corner, not an earlier crossing's
        for _, road in roads:
            # the best rectangle for the links of one road: ONE rectangle for every piece of it. Its axis the road's (the covered lanes' directions),
            # ``width`` metres along it, centred on the middle of the crossing, and across it from the first covered lane's outer edge to the last
            # one's, over the median too
            axes = [t for *_, t in road]
            ends = []
            for part in parts:
                ss = [(s0, s1) for _, p2, s0, s1, _t in road if p2 is part]
                if not ss:                                      # a piece with only the other road's lanes
                    continue
                ends += [part.interpolate(min(a for a, _ in ss)), part.interpolate(max(b for _, b in ss))]
            ux0, uy0 = axes[0]
            sx = sum(ax if ax * ux0 + ay * uy0 >= 0 else -ax for ax, ay in axes)
            sy = sum(ay if ax * ux0 + ay * uy0 >= 0 else -ay for ax, ay in axes)
            nrm = math.hypot(sx, sy) or 1.0
            ux, uy = sx / nrm, sy / nrm
            vx, vy = -uy, ux
            vs = [p.x * vx + p.y * vy for p in ends]
            v_lo, v_hi = min(vs), max(vs)
            u_c = sum(p.x * ux + p.y * uy for p in ends) / len(ends)
            if v_hi - v_lo < 1.0:
                continue
            width = _zebra_width(c[4], v_hi - v_lo)
            rect = Polygon([(ux * (u_c + su * width / 2) + vx * vv, uy * (u_c + su * width / 2) + vy * vv)
                            for su, vv in ((1, v_lo), (1, v_hi), (-1, v_hi), (-1, v_lo))])
            if placed is not None and rect.intersection(placed).area > _MAX_OVERLAP * rect.area:
                continue
            # 3. the rectangle projected on each lane, as it really overlaps it: the lane's stretch is where the rectangle meets the lane's own
            #    shape (so a lane that is short, or ends inside the zebra, still has its part), and the part across the lane is the overlap's extent
            #    across the rectangle (in its own frame, so the stripes stay in step from lane to lane)
            rows = []
            on_rect = [j for j in tree.query(rect) if (c[7] is None or lane_level[j] == c[7])]    # every lane the RECTANGLE overlaps, not only those the line touches:
            for j in on_rect:                                                      # a lane that goes on past the line's lane is part of the zebra
                lane = geom[j]
                ip = rect.intersection(poly[j])
                if ip.is_empty or ip.area < 0.2:
                    continue
                pts = [pt for g in getattr(ip, "geoms", [ip]) if g.geom_type == "Polygon" for pt in g.exterior.coords]
                if not pts:
                    continue
                lrs = [lane.project(Point(pt)) for pt in pts]
                a, b = min(lrs), max(lrs)
                vv = [x * vx + y * vy for x, y in pts]
                ov_lo, ov_hi = max(min(vv), v_lo), min(max(vv), v_hi)
                if b - a < 0.3 or ov_hi - ov_lo < 0.1:
                    continue
                mid = ip.centroid
                tx, ty = tangent(lane, mid)
                if abs(tx * ux + ty * uy) < (_MIN_ALONG_CONNECTOR if lane_conn[j] else _MIN_ALONG):                          # a lane that does not run along the road (a side road): not under it
                    continue
                nv = -ty * vx + tx * vy                                        # > 0: going on across the zebra goes to the lane's left
                rows.append((lane_id[j], link_id[j], a, b, ov_lo - v_lo, ov_hi - v_lo, nv > 0))
            if not rows:
                continue
            n = seen.get((src, osm), 0)
            seen[(src, osm)] = n + 1
            cid = f"{src}{osm}" + (f"#{n + 1}" if n else "")
            fresh.append(rect)
            crossing_rows.append((cid, "way" if src == "w" else "node", ct, mk, _painted(ct, mk), width, v_hi - v_lo, osm,
                                  Polygon([to_ll(x, y) for x, y in rect.exterior.coords]).wkt))
            lane_rows += [(cid, *r) for r in rows]
        for rect in fresh:
            placed = rect if placed is None else placed.union(rect)
    if crossing_rows:
        con.executemany(f"INSERT INTO {sch}.crossing SELECT ?::VARCHAR, ?::VARCHAR, ?::VARCHAR, ?::VARCHAR, ?::BOOLEAN, ?::DOUBLE, ?::DOUBLE, "
                        f"?::BIGINT, ST_GeomFromText(?::VARCHAR)", crossing_rows)
        con.executemany(f"INSERT INTO {sch}.lane_crossing VALUES (?::VARCHAR, ?::VARCHAR, ?::BIGINT, ?::DOUBLE, ?::DOUBLE, ?::DOUBLE, ?::DOUBLE, ?::BOOLEAN)",
                        lane_rows)
