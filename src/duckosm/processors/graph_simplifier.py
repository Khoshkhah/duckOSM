"""
Graph simplifier processor - contracts degree-2 nodes and splits ways at junctions.
"""

import math
import time
import logging
from duckosm.processors.base import BaseProcessor

logger = logging.getLogger(__name__)

# Node-refs processed per batch keeps the per-segment aggregate + geometry build
# within ~16 GB. Used only when batches are chosen automatically.
NODE_REFS_PER_BATCH = 13_000_000


class GraphSimplifier(BaseProcessor):
    """
    Simplifies the road network by contracting degree-2 nodes.

    This results in a graph where:
    1. Every node is a junction (degree != 2) or an endpoint.
    2. Edges contain the full geometry (stitching intermediate points).
    3. Length and costs are correctly aggregated.

    For very large (country-scale) extracts the simplified edges are built in
    several way-id buckets so the heavy geometry aggregation stays within memory.
    """

    def __init__(self, con, batches: int = 0, merge_segments: bool = False):
        super().__init__(con)
        # 0 = auto (pick from node count), 1 = single pass, N>1 = N buckets.
        self.batches = batches
        # Contract degree-2 chains of consecutive segments that share the same osm_id.
        self.merge_segments = merge_segments

    def run(self) -> None:
        """Run the simplification pipeline."""
        logger.info("Simplifying graph...")
        start = time.time()

        # 1. Identify junction nodes (global; junction-ness spans ways)
        self._find_junctions()

        # 2 + 3. Segment ways and build simplified edges, optionally in batches
        num_batches = self._num_batches()
        if num_batches > 1:
            logger.info(f"  Building simplified edges in {num_batches} way-id batches")
        offset = 0
        for b in range(num_batches):
            way_filter = "TRUE" if num_batches == 1 else f"(wn.way_id % {num_batches}) = {b}"
            self._segment_ways(way_filter)
            self._build_simplified_edges(create=(b == 0), edge_offset=offset)
            offset = self.fetchone(
                "SELECT COALESCE(MAX(edge_id), 0) FROM simplified_edges_forward")[0]
            if num_batches > 1:
                logger.info(f"    batch {b + 1}/{num_batches}: {offset:,} edges so far")

        # 4. Split self-loops with virtual midpoint nodes
        self._split_self_loops()

        # 4b. Optionally contract degree-2 chains of same-osm_id segments (forward edges,
        #     before reverse edges + the stable re-key are built).
        if self.merge_segments:
            self._contract_chains()

        # 5. Finalize tables
        self._finalize_tables()

        # 6. Replace the position-based edge_id with a stable content hash so rebuilds
        #    don't renumber the whole graph (see _rekey_edges).
        self._rekey_edges()

        elapsed = time.time() - start
        logger.info(f"  Graph simplified in {elapsed:.2f}s")

    def _rekey_edges(self) -> None:
        """Replace the row-number edge_id with a deterministic hash of the edge's identity
        (osm_id, source, target, is_reverse).

        Position-based ids (row_number) change on every rebuild, forcing every downstream
        consumer keyed on edge_id (map-matching, joins, the regime/simulation pipelines) to
        re-run and re-match. A content hash is STABLE: an unchanged edge keeps its id across
        rebuilds, so only added/removed/changed edges shift. BIGINT holds the 63-bit hash.
        """
        # De-duplicate, then re-key. (osm_id, source, target, is_reverse) is unique EXCEPT for
        # self-crossing ways that pass the same junction pair twice in the SAME direction — e.g.
        # a lead-in chord plus a loop arc, both A->B. (A genuine loop splits into A->B and B->A:
        # opposite orientation, distinct keys, so it is never affected here.) For such a
        # same-direction parallel, the longer arc is never the optimal route between the two
        # junctions — its interior nodes aren't junctions, so nothing branches off it — so keep
        # the shortest. This makes the natural key unique with no addition to it.
        self.execute("""
            CREATE OR REPLACE TABLE edges AS
            SELECT
                (hash(osm_id, source, target, is_reverse) >> 1)::BIGINT AS edge_id,
                source, target, osm_id, highway, name, maxspeed, oneway, lanes,
                surface, junction, refs, geometry, is_reverse, length_m
            FROM (
                SELECT * EXCLUDE (edge_id), row_number() OVER (
                    PARTITION BY osm_id, source, target, is_reverse
                    ORDER BY length_m, refs::VARCHAR) AS _rn
                FROM edges
            ) WHERE _rn = 1
        """)
        dup = self.fetchone("SELECT COUNT(*) - COUNT(DISTINCT edge_id) FROM edges")[0]
        if dup:
            raise RuntimeError(f"edge_id still has {dup} duplicate ids after de-dup — unexpected.")

    def _num_batches(self) -> int:
        """Number of way-id buckets to build the simplified graph in."""
        if self.batches and self.batches > 0:
            return self.batches
        n_refs = self.fetchone("SELECT COUNT(*) FROM way_nodes")[0] or 0
        return max(1, math.ceil(n_refs / NODE_REFS_PER_BATCH))

    def _find_junctions(self) -> None:
        """Identify nodes that are junctions (degree != 2) or endpoints."""
        self.execute("""
            CREATE OR REPLACE TEMP TABLE node_counts AS
            SELECT 
                node_id,
                COUNT(*) as way_count,
                SUM(CASE WHEN is_endpoint THEN 1 ELSE 0 END) as endpoint_count
            FROM (
                SELECT 
                    node_id,
                    (seq = 0 OR seq = max_seq) AS is_endpoint
                FROM (
                    SELECT 
                        node_id, 
                        seq,
                        MAX(seq) OVER (PARTITION BY way_id) as max_seq
                    FROM way_nodes
                )
            )
            GROUP BY node_id
        """)
        
        self.execute("""
            CREATE OR REPLACE TABLE junctions AS
            SELECT node_id
            FROM node_counts
            WHERE way_count > 1        -- Shared between ways
               OR endpoint_count > 0   -- Endpoint of a way
        """)
        
        junction_count = self.fetchone("SELECT COUNT(*) FROM junctions")[0]
        logger.info(f"  Identified {junction_count:,} junction nodes")

    def _segment_ways(self, way_filter: str = "TRUE") -> None:
        """Split ways into segments between junctions, ensuring junctions are shared.

        A segment runs from one junction to the next; junction nodes are shared between
        the two adjacent segments (they are the end of one and the start of the next).
        ``segment_idx`` is the sequence index of the segment's starting junction.

        This is computed with O(N) window functions over the (already ordered) way
        nodes. The earlier implementation joined nodes to segment ranges with an
        inequality (``seq BETWEEN start AND end``), which materialised N*K rows per way
        (N nodes x K segments) and OOM'd on dense, highly-connected networks (cycling).

        ``way_filter`` is a SQL predicate on ``wn.way_id`` used to restrict to one
        batch of ways; partitioning by way_id keeps each way's nodes together so the
        window functions stay correct.
        """
        self.execute(f"""
            CREATE OR REPLACE TABLE way_segments AS
            WITH marked AS (
                SELECT
                    wn.way_id,
                    wn.node_id,
                    wn.seq,
                    (j.node_id IS NOT NULL) AS is_junction,
                    -- latest junction at or before this node = start of its segment
                    max(CASE WHEN j.node_id IS NOT NULL THEN wn.seq END) OVER (
                        PARTITION BY wn.way_id ORDER BY wn.seq
                        ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS seg_start_incl,
                    -- nearest junction strictly before / after (segment links for junction nodes)
                    max(CASE WHEN j.node_id IS NOT NULL THEN wn.seq END) OVER (
                        PARTITION BY wn.way_id ORDER BY wn.seq
                        ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING) AS prev_junction,
                    min(CASE WHEN j.node_id IS NOT NULL THEN wn.seq END) OVER (
                        PARTITION BY wn.way_id ORDER BY wn.seq
                        ROWS BETWEEN 1 FOLLOWING AND UNBOUNDED FOLLOWING) AS next_junction
                FROM way_nodes wn
                LEFT JOIN junctions j ON wn.node_id = j.node_id
                WHERE {way_filter}
            )
            -- Non-junction nodes belong to the single segment that contains them.
            SELECT way_id, seg_start_incl AS segment_idx, node_id, seq
            FROM marked
            WHERE NOT is_junction
            UNION ALL
            -- Junction nodes start the next segment (when one follows).
            SELECT way_id, seq AS segment_idx, node_id, seq
            FROM marked
            WHERE is_junction AND next_junction IS NOT NULL
            UNION ALL
            -- Junction nodes also close the previous segment (when one precedes).
            SELECT way_id, prev_junction AS segment_idx, node_id, seq
            FROM marked
            WHERE is_junction AND prev_junction IS NOT NULL
        """)

    def _build_simplified_edges(self, create: bool = True, edge_offset: int = 0) -> None:
        """Create (or append a batch of) simplified edges by stitching node geometries.

        ``create`` controls whether the target table is created or appended to;
        ``edge_offset`` shifts the row-numbered edge_id so ids stay unique across
        batches.
        """
        verb = ("CREATE OR REPLACE TABLE simplified_edges_forward AS"
                if create else "INSERT INTO simplified_edges_forward")
        # Stitch into LineStrings and calculate length
        self.execute(f"""
            {verb}
            WITH segment_groups AS (
                -- Aggregate ONLY the numeric node geometry here, keyed by segment. Per-way
                -- attributes (highway/name/maxspeed/oneway/lanes/surface/junction) are
                -- attached afterwards by joining the small `ways` table, so this heavy
                -- ordered aggregate never carries the OSM tags MAP. Replicating that MAP
                -- across ~26M node rows is what OOM'd the country-scale cycling network.
                SELECT
                    ws.way_id,
                    ws.segment_idx,
                    LIST(ws.node_id ORDER BY ws.seq) as node_list,
                    LIST(n.lon ORDER BY ws.seq) as lons,
                    LIST(n.lat ORDER BY ws.seq) as lats
                FROM way_segments ws
                JOIN raw.nodes n ON ws.node_id = n.osm_id
                GROUP BY ws.way_id, ws.segment_idx
            )
            SELECT
                ({edge_offset} + row_number() OVER ())::INTEGER AS edge_id,
                sg.node_list[1] AS source,
                sg.node_list[len(sg.node_list)] AS target,
                sg.way_id AS osm_id,
                w.highway,
                w.name,
                w.maxspeed,
                w.oneway,
                w.lanes_fwd AS lanes,
                w.surface,
                w.junction,
                sg.node_list as refs,
                ST_GeomFromText('LINESTRING(' || list_aggregate(
                    list_transform(range(1, len(sg.lons) + 1), i -> sg.lons[i] || ' ' || sg.lats[i]),
                    'string_agg', ', '
                ) || ')') AS geometry,
                FALSE AS is_reverse,
                -- Haversine length summed over consecutive vertices, computed directly
                -- from the coordinate lists. Avoids round-tripping through the geometry
                -- (ST_PointN per point is O(n^2) and replicates the geometry blob per
                -- vertex, which OOMs on dense country-scale networks).
                CAST(list_sum(list_transform(
                    range(1, len(sg.lons)),
                    i -> 12742000 * ASIN(SQRT(
                        POWER(SIN(RADIANS(sg.lats[i + 1] - sg.lats[i]) / 2), 2) +
                        COS(RADIANS(sg.lats[i])) * COS(RADIANS(sg.lats[i + 1])) *
                        POWER(SIN(RADIANS(sg.lons[i + 1] - sg.lons[i]) / 2), 2)
                    ))
                )) AS FLOAT) AS length_m
            FROM segment_groups sg
            JOIN ways w ON w.osm_id = sg.way_id
            WHERE len(sg.lons) >= 2
        """)


    def _split_self_loops(self) -> None:
        """Split self-loop edges (source == target) by inserting a virtual midpoint node."""
        # Count self-loops
        loop_count = self.fetchone("SELECT COUNT(*) FROM simplified_edges_forward WHERE source = target")[0]
        if loop_count == 0:
            return
        
        logger.info(f"  Splitting {loop_count} self-loop edges...")

        # Get max edge_id to assign new IDs
        max_edge_id = self.fetchone("SELECT MAX(edge_id) FROM simplified_edges_forward")[0] or 0

        self.execute(f"""
            CREATE OR REPLACE TEMP TABLE split_loops AS
            WITH loops AS (
                SELECT
                    row_number() OVER (ORDER BY edge_id) AS loop_row,
                    edge_id,
                    source,
                    osm_id,
                    highway,
                    name,
                    maxspeed,
                    oneway,
                    lanes,
                    surface,
                    junction,
                    refs,
                    geometry,
                    length_m,
                    -- Deterministic virtual midpoint-node id (was -(edge_id), which depended on
                    -- the volatile row-number edge_id). A loop is keyed by (osm_id, source);
                    -- a stable negative id keeps the split edges reproducible across rebuilds.
                    -((hash(osm_id, source) >> 2)::BIGINT) AS virtual_node_id,
                    ST_PointN(geometry, (ST_NPoints(geometry) / 2 + 1)::INTEGER) AS midpoint_geom
                FROM simplified_edges_forward
                WHERE source = target
                  AND ST_NPoints(geometry) >= 3
            )
            -- First half: source -> midpoint
            SELECT
                ({max_edge_id} + loop_row * 2 - 1)::INTEGER AS edge_id,
                source,
                virtual_node_id AS target,
                osm_id,
                highway,
                name,
                maxspeed,
                oneway,
                lanes,
                surface,
                junction,
                refs[1:len(refs)/2 + 1] as refs,
                ST_AsText(ST_LineSubstring(geometry, 0, 0.5)) AS geometry_text,
                FALSE AS is_reverse,
                length_m / 2 AS length_m,
                virtual_node_id,
                ST_AsText(midpoint_geom) AS midpoint_text
            FROM loops
            UNION ALL
            -- Second half: midpoint -> source (since source == target for loops)
            SELECT
                ({max_edge_id} + loop_row * 2)::INTEGER AS edge_id,
                virtual_node_id AS source,
                loops.source AS target,
                osm_id,
                highway,
                name,
                maxspeed,
                oneway,
                lanes,
                surface,
                junction,
                refs[len(refs)/2 + 1:] as refs,
                ST_AsText(ST_LineSubstring(geometry, 0.5, 1)) AS geometry_text,
                FALSE AS is_reverse,
                length_m / 2 AS length_m,
                virtual_node_id,
                ST_AsText(midpoint_geom) AS midpoint_text
            FROM loops
        """)

        # Store virtual nodes
        self.execute("""
            CREATE OR REPLACE TEMP TABLE virtual_nodes AS
            SELECT DISTINCT
                virtual_node_id AS node_id,
                ST_GeomFromText(midpoint_text) AS geom
            FROM split_loops
            WHERE source > 0
        """)

        # Rebuild simplified_edges_forward as a new table to avoid DuckDB MVCC/GEOMETRY
        # bug where INSERT INTO a table that was previously ALTER'd + UPDATE'd fails to commit
        self.execute("""
            CREATE TABLE simplified_edges_forward_new AS
            SELECT edge_id, source, target, osm_id, highway, name, maxspeed, oneway, lanes, surface,
                   junction, refs, geometry, is_reverse, length_m
            FROM simplified_edges_forward
            WHERE source != target
            UNION ALL
            SELECT edge_id, source, target, osm_id, highway, name, maxspeed, oneway, lanes, surface,
                   junction, refs, ST_GeomFromText(geometry_text) AS geometry, is_reverse, length_m
            FROM split_loops
            WHERE geometry_text IS NOT NULL
              AND ST_NPoints(ST_GeomFromText(geometry_text)) >= 2
        """)
        self.execute("DROP TABLE simplified_edges_forward")
        self.execute("ALTER TABLE simplified_edges_forward_new RENAME TO simplified_edges_forward")

    def _contract_chains(self) -> None:
        """Merge maximal chains of consecutive forward segments that are the SAME road.

        A node is a contraction point when it has exactly two forward edges that agree on
        EVERY carried attribute (highway, name, oneway, maxspeed, lanes, surface, junction),
        it is a real node (id > 0, so virtual self-loop midpoints stay intact), and the two
        edges pass through it (one in, one out). This merges a street that OSM split into
        several way objects (different osm_id) — the common case — without blurring any
        attribute, since they are all required equal.

        Each maximal run collapses to one edge whose geometry/length/refs are rebuilt from
        the stitched node sequence. The merged edge keeps its LONGEST member's osm_id as the
        representative, so the stable edge_id hash (osm_id, source, target, is_reverse) stays
        well-defined. Runs on forward edges only, before reverse edges and the re-key.
        """
        # Each forward edge as two half-edges (at its source, at its target), carrying the
        # attributes the merge predicate compares.
        self.execute("""
            CREATE OR REPLACE TEMP TABLE _inc AS
            SELECT edge_id, source AS node, target AS other, refs, TRUE  AS fwd,
                   highway, name, oneway, maxspeed, lanes, surface, junction
            FROM simplified_edges_forward
            UNION ALL
            SELECT edge_id, target AS node, source AS other, refs, FALSE AS fwd,
                   highway, name, oneway, maxspeed, lanes, surface, junction
            FROM simplified_edges_forward
        """)
        # Contraction nodes: degree-2, real, through-orientation (one in/out), and the two
        # edges identical on every attribute (chr(1) sentinel so NULL == NULL groups as one).
        self.execute("""
            CREATE OR REPLACE TEMP TABLE _node2 AS
            SELECT node FROM _inc
            WHERE node > 0
            GROUP BY node
            HAVING count(*) = 2 AND sum(fwd::INT) = 1
               AND count(DISTINCT highway) = 1
               AND count(DISTINCT coalesce(name, chr(1))) = 1
               AND count(DISTINCT oneway) = 1
               AND count(DISTINCT coalesce(maxspeed, chr(1))) = 1
               AND count(DISTINCT coalesce(CAST(lanes AS VARCHAR), chr(1))) = 1
               AND count(DISTINCT coalesce(surface, chr(1))) = 1
               AND count(DISTINCT coalesce(junction, chr(1))) = 1
        """)
        if self.fetchone("SELECT COUNT(*) FROM _node2")[0] == 0:
            self._empty_edge_id_map()
            self.execute("DROP TABLE IF EXISTS _inc; DROP TABLE IF EXISTS _node2;")
            logger.info("  merge_segments: no same-road chains to contract")
            return

        # Walk each chain from a terminal node through contraction nodes, accumulating the
        # ordered node sequence (refs_acc, source -> target). edge_set guards against revisits.
        self.execute("""
            CREATE OR REPLACE TEMP TABLE _chains AS
            WITH RECURSIVE walk AS (
                SELECT i.node AS start, i.edge_id AS last_edge, i.other AS cur,
                       [i.edge_id] AS edge_set,
                       CASE WHEN i.fwd THEN i.refs ELSE list_reverse(i.refs) END AS refs_acc
                FROM _inc i
                WHERE i.node NOT IN (SELECT node FROM _node2)
                  AND i.other IN (SELECT node FROM _node2)
                UNION ALL
                SELECT w.start, i.edge_id, i.other,
                       list_append(w.edge_set, i.edge_id),
                       list_concat(w.refs_acc,
                           (CASE WHEN i.fwd THEN i.refs ELSE list_reverse(i.refs) END)[2:])
                FROM walk w
                JOIN _node2 n ON n.node = w.cur
                JOIN _inc i ON i.node = w.cur AND i.edge_id <> w.last_edge
                WHERE NOT list_contains(w.edge_set, i.edge_id)
            )
            SELECT row_number() OVER () AS cid, start AS source, cur AS target,
                   edge_set, refs_acc
            FROM walk
            WHERE cur NOT IN (SELECT node FROM _node2)   -- reached the far terminal
              AND len(edge_set) > 1                      -- real (multi-segment) chains only
              AND start < cur                            -- keep one of the two walk directions
        """)

        n_chains = self.fetchone("SELECT COUNT(*) FROM _chains")[0]
        if n_chains == 0:
            self._empty_edge_id_map()
            self.execute("DROP TABLE IF EXISTS _inc; DROP TABLE IF EXISTS _node2; "
                         "DROP TABLE IF EXISTS _chains;")
            logger.info("  merge_segments: no same-road chains to contract")
            return

        # Per-chain metadata: stitched endpoints/refs, uniform attributes (from any member —
        # identical by the same-road predicate), and the representative osm_id = LONGEST member
        # (so the stable edge_id hash stays well-defined for the merged edge).
        self.execute("""
            CREATE OR REPLACE TEMP TABLE _cmeta AS
            WITH members AS (
                SELECT c.cid, e.osm_id, e.length_m
                FROM _chains c, UNNEST(c.edge_set) AS m(eid)
                JOIN simplified_edges_forward e ON e.edge_id = m.eid
            ),
            prim AS (SELECT cid, arg_max(osm_id, length_m) AS osm_id FROM members GROUP BY cid)
            SELECT c.cid, c.source, c.target, c.refs_acc, p.osm_id,
                   e.highway, e.name, e.maxspeed, e.oneway, e.lanes, e.surface, e.junction
            FROM _chains c
            JOIN prim p USING (cid)
            JOIN simplified_edges_forward e ON e.edge_id = c.edge_set[1]
        """)

        # One edge per chain: geometry + length rebuilt from the stitched node coords.
        self.execute("""
            CREATE OR REPLACE TEMP TABLE _merged AS
            WITH coords AS (
                SELECT a.cid,
                       list(n.lon ORDER BY g.i) AS lons,
                       list(n.lat ORDER BY g.i) AS lats
                FROM _cmeta a
                CROSS JOIN range(1, len(a.refs_acc) + 1) AS g(i)
                JOIN raw.nodes n ON n.osm_id = a.refs_acc[g.i]
                GROUP BY a.cid
            )
            SELECT
                a.source, a.target, a.osm_id, a.highway, a.name, a.maxspeed, a.oneway,
                a.lanes, a.surface, a.junction, a.refs_acc AS refs,
                ST_GeomFromText('LINESTRING(' || list_aggregate(
                    list_transform(range(1, len(c.lons) + 1), i -> c.lons[i] || ' ' || c.lats[i]),
                    'string_agg', ', ') || ')') AS geometry,
                FALSE AS is_reverse,
                CAST(list_sum(list_transform(range(1, len(c.lons)),
                    i -> 12742000 * ASIN(SQRT(
                        POWER(SIN(RADIANS(c.lats[i + 1] - c.lats[i]) / 2), 2) +
                        COS(RADIANS(c.lats[i])) * COS(RADIANS(c.lats[i + 1])) *
                        POWER(SIN(RADIANS(c.lons[i + 1] - c.lons[i]) / 2), 2)
                    )))) AS FLOAT) AS length_m
            FROM _cmeta a JOIN coords c USING (cid)
            WHERE len(c.lons) >= 2
        """)

        # Matching table (v1 merge-off id -> v2 merged id), one row per original segment.
        # seq = the segment's position along the road (forward direction). Stable-hash ids on
        # both sides (same formula as _rekey_edges), forward + reverse (reverse only two-way).
        # Edges NOT in this table are unchanged by the merge.
        self.execute("""
            CREATE OR REPLACE TABLE edge_id_map AS
            WITH mem AS (
                SELECT c.cid, m.seq, e.osm_id, e.source, e.target
                FROM _chains c, UNNEST(c.edge_set) WITH ORDINALITY AS m(eid, seq)
                JOIN simplified_edges_forward e ON e.edge_id = m.eid
            )
            SELECT (hash(mem.osm_id, mem.source, mem.target, FALSE) >> 1)::BIGINT AS old_edge_id,
                   (hash(cm.osm_id, cm.source, cm.target, FALSE) >> 1)::BIGINT AS new_edge_id,
                   mem.seq::INTEGER AS seq, FALSE AS is_reverse
            FROM mem JOIN _cmeta cm USING (cid)
            UNION ALL
            SELECT (hash(mem.osm_id, mem.target, mem.source, TRUE) >> 1)::BIGINT,
                   (hash(cm.osm_id, cm.target, cm.source, TRUE) >> 1)::BIGINT,
                   mem.seq::INTEGER, TRUE
            FROM mem JOIN _cmeta cm USING (cid)
            WHERE NOT cm.oneway
        """)

        # Forward edges = untouched singletons + one edge per merged chain.
        self.execute("""
            CREATE OR REPLACE TABLE simplified_edges_forward AS
            SELECT (row_number() OVER ())::INTEGER AS edge_id, *
            FROM (
                SELECT source, target, osm_id, highway, name, maxspeed, oneway, lanes,
                       surface, junction, refs, geometry, is_reverse, length_m FROM _merged
                UNION ALL
                SELECT source, target, osm_id, highway, name, maxspeed, oneway, lanes,
                       surface, junction, refs, geometry, is_reverse, length_m
                FROM simplified_edges_forward
                WHERE edge_id NOT IN (SELECT UNNEST(edge_set) FROM _chains)
            )
        """)

        n_removed = self.fetchone(
            "SELECT COALESCE(SUM(len(edge_set)), 0) - COUNT(*) FROM _chains")[0]
        n_map = self.fetchone("SELECT COUNT(*) FROM edge_id_map")[0]
        logger.info(f"  merge_segments: contracted {n_chains:,} chain(s), removed "
                    f"{n_removed:,} forward edge(s); edge_id_map has {n_map:,} row(s)")
        self.execute("DROP TABLE IF EXISTS _inc; DROP TABLE IF EXISTS _node2; "
                     "DROP TABLE IF EXISTS _chains; DROP TABLE IF EXISTS _merged; "
                     "DROP TABLE IF EXISTS _cmeta;")

    def _empty_edge_id_map(self) -> None:
        """Create an empty edge_id_map so the table always exists when merge_segments is on."""
        self.execute("""
            CREATE OR REPLACE TABLE edge_id_map (
                old_edge_id BIGINT, new_edge_id BIGINT, seq INTEGER, is_reverse BOOLEAN)
        """)

    def _finalize_tables(self) -> None:
        """Replace edges and nodes with simplified versions."""
        # 1. Add reverse edges
        max_id = self.fetchone("SELECT MAX(edge_id) FROM simplified_edges_forward")[0] or 0
        
        self.execute(f"""
            CREATE OR REPLACE TABLE simplified_edges AS
            SELECT * FROM simplified_edges_forward
            UNION ALL
            SELECT
                ({max_id} + row_number() OVER ())::INTEGER AS edge_id,
                sef.target AS source,
                sef.source AS target,
                sef.osm_id,
                sef.highway,
                sef.name,
                sef.maxspeed,
                sef.oneway,
                -- Reverse edge carries the backward lane count.
                w.lanes_bwd AS lanes,
                sef.surface,
                sef.junction,
                list_reverse(sef.refs) as refs,
                -- Native reverse geometry
                ST_Reverse(sef.geometry),
                TRUE AS is_reverse,
                sef.length_m
            FROM simplified_edges_forward sef
            JOIN ways w ON w.osm_id = sef.osm_id
            -- Two-way roads get a reverse edge. oneway is the single source of truth
            -- (roundabouts were already normalised to oneway=TRUE upstream).
            WHERE NOT sef.oneway
        """)
        
        # 3. Replace the main edges table
        self.execute("DROP TABLE edges")
        self.execute("ALTER TABLE simplified_edges RENAME TO edges")
        
        # 4. Replace the nodes table to only include junctions + virtual nodes
        self.execute("DROP TABLE nodes")
        self.execute("""
            CREATE TABLE nodes AS
            -- Real OSM nodes
            SELECT 
                rn.osm_id AS node_id,
                ST_Point(rn.lon, rn.lat) AS geom
            FROM raw.nodes rn
            WHERE rn.osm_id IN (SELECT DISTINCT source FROM edges UNION SELECT DISTINCT target FROM edges)
              AND rn.osm_id > 0
        """)
        
        # Add virtual nodes from self-loop splitting (if any exist)
        try:
            self.execute("""
                INSERT INTO nodes
                SELECT node_id, geom FROM virtual_nodes
            """)
            self.execute("DROP TABLE virtual_nodes")
        except Exception:
            pass  # No virtual nodes table if there were no self-loops
        
        # Cleanup
        self.execute("DROP TABLE IF EXISTS junctions")
        self.execute("DROP TABLE IF EXISTS way_segments")
        self.execute("DROP TABLE IF EXISTS simplified_edges_forward")
