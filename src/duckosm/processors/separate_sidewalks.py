"""SeparateSidewalks — tag ``sidewalk=separate`` on a road whose sidewalks OSM draws as their own lines but does not say so.

OSM's way to say "people walk on the sidewalk lines beside this road, not on the road" is ``sidewalk=separate`` on the
road; RoadFilter then leaves the road out of the walking network. Many roads miss that tag although their sidewalks
are mapped (2026-10-10, Monaco's Boulevard du Larvotto: a one-way road with ``footway=sidewalk`` lines on both sides
was still walked on its carriageway, ``walk_type`` ``shared_road``, with a walking-only reverse edge).

The rule: a car road with NO sidewalk tag of any kind (``sidewalk``, ``sidewalk:*``) and no explicit foot permission
gets ``sidewalk=separate`` when both its sides are covered, as OSM's ``sidewalk:both=separate`` would say: a side is covered by a
``footway=sidewalk`` line beside it, or by the other half of a dual carriageway (another car road running the same way, near), which
leaves no pavement between the two: both one-way, opposite ways, on one level, the other half on the left of travel. One side with neither keeps the road walkable, as RoadFilter does for ``sidewalk:left/right``. "Beside" is
checked every :data:`STEP_M` metres along the road: a sidewalk line within :data:`SIDE_M` metres of the road line,
running the same way (within :data:`ANGLE_DEG`), with no other car road between the two (so the far sidewalk of a
dual carriageway's other half does not count). A side has a sidewalk when :data:`SHARE` of the road's spots find one.
The tag is written into ``raw.ways`` with ``duckosm:sidewalk=inferred`` beside it, so every inferred road can be listed.
Runs once, before the per-mode loop (RoadFilter reads the tags). ``options.infer_separate_sidewalks`` turns it off.
"""
import logging
import math

import numpy as np

from duckosm.processors.base import BaseProcessor

logger = logging.getLogger("duckosm.separate_sidewalks")

_CAR_ROADS = ("residential", "service", "unclassified", "road", "tertiary", "tertiary_link", "secondary", "secondary_link",
              "primary", "primary_link", "trunk", "trunk_link", "motorway", "motorway_link", "living_street")
_WALKED = ("residential", "service", "unclassified", "road", "tertiary", "tertiary_link", "secondary", "secondary_link",
           "primary", "primary_link", "trunk", "trunk_link")      # the classes walked on their carriageway without a sidewalk tag
SIDE_M = 12.0      # a sidewalk line this near the road line can be its sidewalk
STEP_M = 5.0       # one spot every this many metres along the road
ANGLE_DEG = 30.0   # the sidewalk runs within this angle of the road
SHARE = 0.6        # a side has a sidewalk when this share of the road's spots find one there


class SeparateSidewalks(BaseProcessor):
    """Write ``sidewalk=separate`` (+ ``duckosm:sidewalk=inferred``) into ``raw.ways`` for roads whose both sides are covered (a sidewalk line or the other carriageway)."""

    def run(self) -> int:
        import shapely
        from shapely import STRtree

        q = lambda vals: "(" + ", ".join(f"'{v}'" for v in vals) + ")"      # noqa: E731
        hw = "map_extract(tags, 'highway')[1]"
        rows = self.fetchall(f"""
            WITH w AS (
                SELECT osm_id, refs, CASE WHEN {hw} = 'footway' AND map_extract(tags, 'footway')[1] = 'sidewalk' THEN 2
                    WHEN {hw} IN {q(_WALKED)} AND COALESCE(map_extract(tags, 'foot')[1], '') NOT IN ('yes', 'designated', 'permissive')
                         AND NOT list_has_any(map_keys(tags), ['sidewalk', 'sidewalk:both', 'sidewalk:left', 'sidewalk:right']) THEN 1
                    ELSE 0 END AS kind,
                CASE WHEN map_extract(tags, 'oneway')[1] IN ('yes', 'true', '1') OR map_extract(tags, 'junction')[1] = 'roundabout' THEN 1
                     WHEN map_extract(tags, 'oneway')[1] = '-1' THEN -1 ELSE 0 END AS ow,
                COALESCE(TRY_CAST(map_extract(tags, 'layer')[1] AS INTEGER),
                         CASE WHEN COALESCE(map_extract(tags, 'tunnel')[1], 'no') <> 'no' THEN -1
                              WHEN COALESCE(map_extract(tags, 'bridge')[1], 'no') <> 'no' THEN 1 ELSE 0 END) AS lvl
                FROM raw.ways WHERE {hw} IN {q(_CAR_ROADS)} OR ({hw} = 'footway' AND map_extract(tags, 'footway')[1] = 'sidewalk')),
            p AS (SELECT osm_id, kind, ow, lvl, unnest(refs) AS ref, generate_subscripts(refs, 1) AS i FROM w)
            SELECT p.osm_id, any_value(p.kind), list(n.lon ORDER BY p.i), list(n.lat ORDER BY p.i), any_value(p.ow), any_value(p.lvl)
            FROM p JOIN raw.nodes n ON n.osm_id = p.ref GROUP BY p.osm_id HAVING count(*) >= 2""")
        if not rows or not any(r[1] == 2 for r in rows) or not any(r[1] == 1 for r in rows):
            logger.info("separate sidewalks: none inferred (no sidewalk lines or no untagged roads)")
            return 0
        lat0 = float(np.mean([np.mean(r[3]) for r in rows]))
        kx, ky = 111320.0 * math.cos(math.radians(lat0)), 111320.0      # metres, local: the distances here are a few metres
        ids = np.array([r[0] for r in rows])
        kind = np.array([r[1] for r in rows])
        ow, lvl = np.array([r[4] for r in rows]), np.array([r[5] for r in rows])     # one-way (1, -1 against the line, 0), level (layer / tunnel / bridge)
        lines = np.array([shapely.linestrings(np.c_[np.array(r[2]) * kx, np.array(r[3]) * ky]) for r in rows])
        walks, cars = np.flatnonzero(kind == 2), np.flatnonzero(kind != 2)
        side_tree, car_tree = STRtree(lines[walks]), STRtree(lines[cars])

        # the spots: every STEP_M along each untagged road (at least its middle), with the road's direction there
        cand = np.flatnonzero(kind == 1)
        road, at = [], []
        for k in cand:
            n = lines[k].length
            s = np.arange(STEP_M / 2, n, STEP_M) if n > STEP_M else np.array([n / 2])
            road += [k] * len(s)
            at += list(s)
        road, at = np.array(road), np.array(at)
        pt = shapely.line_interpolate_point(lines[road], at)
        ahead = shapely.line_interpolate_point(lines[road], np.minimum(at + 0.5, shapely.length(lines[road])))
        back = shapely.line_interpolate_point(lines[road], np.maximum(at - 0.5, 0.0))
        t = shapely.get_coordinates(ahead) - shapely.get_coordinates(back)
        t /= np.linalg.norm(t, axis=1, keepdims=True).clip(1e-9)

        # each spot against each sidewalk line near it: the nearest point, its side, the sidewalk's direction there
        i, j = side_tree.query(pt, predicate="dwithin", distance=SIDE_M)
        sw = lines[walks][j]
        loc = shapely.line_locate_point(sw, pt[i])
        near = shapely.line_interpolate_point(sw, loc)
        v = shapely.get_coordinates(near) - shapely.get_coordinates(pt[i])
        side = np.sign(t[i, 0] * v[:, 1] - t[i, 1] * v[:, 0])                 # + left of the road's direction, - right
        d = shapely.get_coordinates(shapely.line_interpolate_point(sw, loc + 0.5)) - shapely.get_coordinates(shapely.line_interpolate_point(sw, np.maximum(loc - 0.5, 0)))
        d /= np.linalg.norm(d, axis=1, keepdims=True).clip(1e-9)
        ok = (np.abs((d * t[i]).sum(axis=1)) >= math.cos(math.radians(ANGLE_DEG))) & (side != 0) & (lvl[walks][j] == lvl[road[i]])   # same level: not a footway on a bridge above
        # no other car road between the spot and the sidewalk (a dual carriageway's other half, a parallel street)
        gap = shapely.linestrings(np.stack([shapely.get_coordinates(pt[i]), shapely.get_coordinates(near)], axis=1))
        a, b = car_tree.query(gap[ok], predicate="intersects")
        own = cars[b] == road[i[ok]][a]
        blocked = np.zeros(ok.sum(), dtype=bool)
        blocked[a[~own]] = True
        hit = np.flatnonzero(ok)[~blocked]

        left = np.zeros(len(pt), dtype=bool)
        right = np.zeros(len(pt), dtype=bool)
        left[i[hit][side[hit] > 0]] = True
        right[i[hit][side[hit] < 0]] = True
        # a side that faces the other half of a dual carriageway (another car road running the same way, near) has no
        # pavement of its own to walk: covered too (2026-10-10, Boulevard du Larvotto: sidewalk outside, other half inside)
        ci, cj = car_tree.query(pt, predicate="dwithin", distance=SIDE_M)
        keep = cars[cj] != road[ci]
        ci, cj = ci[keep], cars[cj[keep]]
        cl = shapely.line_locate_point(lines[cj], pt[ci])
        cn = shapely.line_interpolate_point(lines[cj], cl)
        cv = shapely.get_coordinates(cn) - shapely.get_coordinates(pt[ci])
        cside = np.sign(t[ci, 0] * cv[:, 1] - t[ci, 1] * cv[:, 0])
        cd = shapely.get_coordinates(shapely.line_interpolate_point(lines[cj], cl + 0.5)) - shapely.get_coordinates(shapely.line_interpolate_point(lines[cj], np.maximum(cl - 0.5, 0)))
        cd /= np.linalg.norm(cd, axis=1, keepdims=True).clip(1e-9)
        # both one-way, travelling opposite ways, on one level, the other half on the LEFT of travel (right-hand traffic)
        # ponytail: right-hand traffic only; left-hand countries have the other half on the right
        oc, oo = ow[road[ci]], ow[cj]
        twin = ((cd * t[ci]).sum(axis=1) * oc * oo <= -math.cos(math.radians(ANGLE_DEG))) & (oc != 0) & (oo != 0) & (lvl[cj] == lvl[road[ci]]) \
            & (cside * oc > 0) & (np.linalg.norm(cv, axis=1) > 1.0)          # > 1 m: not a road it meets end to end
        left[ci[twin & (cside > 0)]] = True
        right[ci[twin & (cside < 0)]] = True
        spots = np.bincount(road, minlength=len(lines)).clip(1)
        both = (np.bincount(road, weights=left, minlength=len(lines)) / spots >= SHARE) & \
               (np.bincount(road, weights=right, minlength=len(lines)) / spots >= SHARE) & (kind == 1)
        found = [int(x) for x in ids[both]]
        if found:
            self.con.execute("CREATE OR REPLACE TEMP TABLE _sidewalk_separate AS SELECT unnest(?::BIGINT[]) AS osm_id", [found])
            self.execute("""UPDATE raw.ways SET tags = map_concat(tags, MAP {'sidewalk': 'separate', 'duckosm:sidewalk': 'inferred'})
                            WHERE osm_id IN (SELECT osm_id FROM _sidewalk_separate)""")
            self.execute("DROP TABLE _sidewalk_separate")
        logger.info(f"separate sidewalks: {len(found)} of {len(cand)} untagged roads have a sidewalk line (or the other carriageway) on both sides: "
                    "tagged sidewalk=separate (duckosm:sidewalk=inferred), walked on their sidewalks")
        return len(found)
