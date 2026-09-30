"""
Path connectivity repair — reconnect dangling cycleway/footway ends to the network.

OSM paths often meet roads without a shared node, so the pedestrian/cycle graph fragments and the
component filter later drops the isolated pieces. This step adds a straight *connector* edge from each
dangling path end to the nearest node within `snap_m`, BEFORE the component filter, so the path joins
the largest component (and becomes routable). Cycling/walking only.

A connector's osm_id is **`-min(a, b)`**, where a/b are the osm_ids of the two roads it joins (the
dangling path and the road at the target node). Negative marks it synthetic (`osm_id < 0`, never
collides with real OSM ids) and points back to the connected roads. edge_id is the usual
`hash(osm_id, source, target)>>1`. Runs on the active schema's `edges`/`nodes` (after
`simplify_graph`, before `build_edge_graph`). See docs/design/connectivity_repair.md.
"""
import logging
import math

from duckosm.processors.base import BaseProcessor

logger = logging.getLogger("duckosm")

# path-like highways whose dead-ends we try to reconnect
_PATH = ("cycleway", "footway", "path", "steps", "pedestrian", "track")


class PathConnector(BaseProcessor):
    def __init__(self, con, snap_m=10.0):
        super().__init__(con)
        self.snap_m = float(snap_m)

    def run(self) -> None:
        # 1. Undirected segments (forward+reverse collapse to one) with their osm_id + highway.
        self.execute("""
            CREATE OR REPLACE TEMP TABLE _und AS
            SELECT least(source, target) AS a, greatest(source, target) AS b,
                   min(osm_id) AS osm_id, any_value(highway) AS hw
            FROM edges GROUP BY least(source, target), greatest(source, target)
        """)
        # 2. Dangles: a node touching exactly ONE segment, and that segment is a path.
        dangles = self.fetchall(f"""
            WITH inc AS (
                SELECT a AS node, b AS other, osm_id, hw FROM _und
                UNION ALL SELECT b AS node, a AS other, osm_id, hw FROM _und
            ), deg AS (
                SELECT node, count(*) AS d, any_value(other) AS other,
                       any_value(osm_id) AS osm_id, any_value(hw) AS hw
                FROM inc GROUP BY node
            )
            SELECT deg.node, deg.other, deg.osm_id, deg.hw, ST_X(n.geom), ST_Y(n.geom)
            FROM deg JOIN nodes n ON n.node_id = deg.node
            WHERE deg.d = 1 AND deg.hw IN {_PATH}
        """)
        if not dangles:
            logger.info("  PathConnector: no dangling path ends")
            return

        # each node's road osm_id = min osm_id of the segments touching it (for the -min(a,b) rule)
        node_osm = {r[0]: r[1] for r in self.fetchall("""
            SELECT node, min(osm_id) FROM (
                SELECT a AS node, osm_id FROM _und UNION ALL SELECT b AS node, osm_id FROM _und
            ) GROUP BY node""")}

        # 3. Nearest node within snap_m (local metric), excluding self + the dangle's own other end.
        import numpy as np
        nodes = self.fetchall("SELECT node_id, ST_X(geom), ST_Y(geom) FROM nodes")
        nid = np.array([r[0] for r in nodes], dtype=np.int64)
        nlon = np.array([r[1] for r in nodes], dtype=np.float64)
        nlat = np.array([r[2] for r in nodes], dtype=np.float64)

        pairs, seen = [], set()
        for dnode, other, da_osm, hw, dlon, dlat in dangles:
            coslat = math.cos(math.radians(dlat)) or 1e-9
            dist = np.hypot((nlon - dlon) * 111320.0 * coslat, (nlat - dlat) * 111320.0)
            dist[(nid == dnode) | (nid == other)] = np.inf      # not self, not own other-end
            j = int(np.argmin(dist))
            if dist[j] <= self.snap_m:
                t = int(nid[j])
                key = (min(dnode, t), max(dnode, t))
                if key not in seen:                             # one connector per node-pair
                    seen.add(key)
                    osm_id = -min(int(da_osm), int(node_osm.get(t, da_osm)))   # -min(a, b)
                    pairs.append((int(dnode), t, str(hw), round(float(dist[j]), 2), osm_id))
        if not pairs:
            logger.info(f"  PathConnector: no dangle within {self.snap_m:g} m of the network")
            return

        # 4. Add connector edges (both directions); edge_id via the standard hash.
        self.execute("CREATE OR REPLACE TEMP TABLE _pairs "
                     "(source BIGINT, target BIGINT, highway VARCHAR, length_m DOUBLE, osm_id BIGINT)")
        self.con.executemany("INSERT INTO _pairs VALUES (?, ?, ?, ?, ?)", pairs)
        self.execute("""
            INSERT INTO edges (edge_id, edge_ref, source, target, osm_id, highway, name, oneway, lanes,
                               surface, access, junction, layer, bridge, tunnel, service, refs,
                               geometry, is_reverse, length_m)
            WITH base AS (
                SELECT p.source, p.target, p.highway, p.length_m, p.osm_id, sn.geom AS sgeom, tn.geom AS tgeom
                FROM _pairs p
                JOIN nodes sn ON sn.node_id = p.source
                JOIN nodes tn ON tn.node_id = p.target
            ), dir AS (
                SELECT source, target, highway, length_m, osm_id, sgeom, tgeom, FALSE AS is_reverse FROM base
                UNION ALL
                SELECT target AS source, source AS target, highway, length_m, osm_id,
                       tgeom AS sgeom, sgeom AS tgeom, TRUE AS is_reverse FROM base
            )
            SELECT (hash(osm_id, source, target) >> 1)::BIGINT AS edge_id,
                   osm_id || '#'
                       || dense_rank() OVER (PARTITION BY osm_id
                                             ORDER BY least(source, target), greatest(source, target))
                       || (CASE WHEN is_reverse THEN 'r' ELSE 'f' END) AS edge_ref,
                   -- name, oneway, lanes, surface, access, junction, layer, bridge, tunnel,
                   -- service: a snap connector has none of them. Positional, so this list must
                   -- stay the same length as the column list above.
                   source, target, osm_id, highway, NULL, FALSE, NULL,
                   NULL, NULL, NULL, NULL, NULL, NULL, NULL, [source, target]::BIGINT[],
                   ST_MakeLine(sgeom, tgeom), is_reverse, length_m
            FROM dir
        """)
        self.execute("DROP TABLE IF EXISTS _pairs")
        self.execute("DROP TABLE IF EXISTS _und")
        logger.info(f"  PathConnector: reconnected {len(pairs):,} dangling path end(s) "
                    f"(+{2*len(pairs):,} connector edges) within {self.snap_m:g} m")
