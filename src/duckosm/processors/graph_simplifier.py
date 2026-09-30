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

# Non-road (non-vehicular) highway classes. A ROAD's split points are decided by ROAD connectivity
# only — a footway/path/cycleway joining a road mid-segment must not fragment it, or the road would
# be segmented differently in the walking/cycling graphs (which contain those paths) than in driving
# (which doesn't), giving the same physical road a different edge_id per mode. These paths still
# split at every junction for their OWN routing.
_NON_ROAD_HIGHWAYS = "('footway','path','cycleway','steps','pedestrian','bridleway','corridor')"
_IS_ROAD = f"COALESCE(w.highway NOT IN {_NON_ROAD_HIGHWAYS}, TRUE)"


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

        # 4c. Break same-direction parallel forward pairs (two arcs of one way, both A->B — a
        #     figure-8 / double-lollipop) by splitting every arc but the shortest with a virtual
        #     node. Runs BEFORE the antiparallel split so it only ever sees whole arcs. See
        #     docs/design/split_same_direction_parallels.md.
        self._split_parallel_pairs()

        # 4d. Break two-way antiparallel forward pairs (A->B and B->A on the same osm_id, incl. the
        #     halves _split_self_loops just produced) by splitting the longer arc with a virtual node.
        #     After the reverse twins are added this keeps every directed (osm_id, source, target)
        #     unique — the precondition for dropping is_reverse from the edge_id hash (step 1).
        self._split_antiparallel_pairs()

        # 5. Finalize tables
        self._finalize_tables()

        # 6. Replace the position-based edge_id with a stable content hash so rebuilds
        #    don't renumber the whole graph (see _rekey_edges).
        self._rekey_edges()

        elapsed = time.time() - start
        logger.info(f"  Graph simplified in {elapsed:.2f}s")

    def _rekey_edges(self) -> None:
        """Replace the row-number edge_id with a deterministic content hash of the edge's identity
        ``(osm_id, source, target)`` — direction-free.

        ``is_reverse`` is NOT in the hash: direction is already given by ``source -> target`` (the
        forward and reverse of a two-way road get distinct ids because their endpoints are swapped),
        and after self-loops / two-way antiparallel arcs are split with virtual nodes
        (``_split_self_loops`` / ``_split_antiparallel_pairs``), ``(osm_id, source, target)`` is
        globally unique, so the flag is redundant here. It stays as a column. See
        ``docs/design/drop_is_reverse_from_edge_id.md``.

        Position-based ids (row_number) change on every rebuild, forcing every downstream consumer
        keyed on edge_id to re-run and re-match. A content hash is STABLE: an unchanged edge keeps its
        id across rebuilds. BIGINT holds the 63-bit hash.
        """
        # After the loop / same-direction-parallel / antiparallel splits, (osm_id, source, target)
        # is unique BY CONSTRUCTION — verify that and FAIL LOUDLY on a residual duplicate rather
        # than silently deleting geometry (the old keep-shortest dedup did exactly that to a
        # figure-8's second arc; see docs/design/split_same_direction_parallels.md). The dedup
        # below stays as defence-in-depth but must remove zero rows.
        dup = self.fetchone("""
            SELECT count(*) FROM (
                SELECT 1 FROM edges GROUP BY osm_id, source, target HAVING count(*) > 1)""")[0]
        if dup:
            ex = self.fetchone("""
                SELECT osm_id, source, target, count(*) FROM edges
                GROUP BY 1, 2, 3 HAVING count(*) > 1 LIMIT 1""")
            raise RuntimeError(
                f"{dup} duplicate (osm_id, source, target) group(s) survived the loop/parallel/"
                f"antiparallel splits — e.g. osm_id={ex[0]} {ex[1]}->{ex[2]} x{ex[3]}. Refusing "
                "to dedup: that would silently delete geometry. "
                "See docs/design/split_same_direction_parallels.md.")
        # edge_ref: a human-readable secondary id '{osm_id}#{seq}{f|r}'. seq numbers the FORWARD edges
        # of a way in TRAVERSAL order (source-end segment = #1); a reverse edge inherits its forward
        # twin's seq, so the two directions of a two-way segment share seq (#2f / #2r) while two
        # distinct one-way arcs of a self-crossing way get different seq. f/r is the direction. Not a
        # join key (positional seq is not rebuild-stable) — the integer edge_id stays that.
        self.execute("""
            CREATE OR REPLACE TABLE edges AS
            WITH deduped AS (
                SELECT * EXCLUDE (_rn) FROM (
                    SELECT * EXCLUDE (edge_id), row_number() OVER (
                        PARTITION BY osm_id, source, target
                        ORDER BY length_m, refs::VARCHAR) AS _rn
                    FROM edges
                ) WHERE _rn = 1
            ),
            fseq AS (
                -- number forward edges in TRAVERSAL order (by the source node's position in the
                -- way's original node list); virtual-node sources (negative, absent from that list)
                -- sort last, deterministically.
                SELECT d.osm_id, d.source, d.target,
                       dense_rank() OVER (PARTITION BY d.osm_id ORDER BY
                           COALESCE(list_position(w.refs, d.source), 2000000000),
                           d.source, d.target) AS seq
                FROM deduped d
                LEFT JOIN raw.ways w ON w.osm_id = d.osm_id
                WHERE NOT d.is_reverse
            )
            SELECT
                (hash(d.osm_id, d.source, d.target) >> 1)::BIGINT AS edge_id,
                d.osm_id || '#' || COALESCE(ff.seq, fr.seq)
                    || (CASE WHEN d.is_reverse THEN 'r' ELSE 'f' END) AS edge_ref,
                d.source, d.target, d.osm_id, d.highway, d.name, d.maxspeed, d.oneway, d.lanes,
                d.surface, d.access, d.junction, d.layer, d.bridge, d.tunnel, d.service, d.refs, d.geometry,
                d.is_reverse, d.length_m
            FROM deduped d
            LEFT JOIN fseq ff ON NOT d.is_reverse
                AND ff.osm_id = d.osm_id AND ff.source = d.source AND ff.target = d.target
            LEFT JOIN fseq fr ON d.is_reverse
                AND fr.osm_id = d.osm_id AND fr.source = d.target AND fr.target = d.source
        """)
        n = self.fetchone("SELECT COUNT(*) FROM edges")[0]
        du = self.fetchone("SELECT COUNT(DISTINCT edge_id) FROM edges")[0]
        dt = self.fetchone("SELECT COUNT(*) FROM (SELECT DISTINCT osm_id, source, target FROM edges)")[0]
        dr = self.fetchone("SELECT COUNT(DISTINCT edge_ref) FROM edges")[0]
        if du != n or dt != n or dr != n:
            raise RuntimeError(
                f"edge id/ref not unique after re-key: {n} edges, {du} distinct edge_id, "
                f"{dt} distinct (osm_id, source, target), {dr} distinct edge_ref — the "
                f"loop/antiparallel splits should make these equal.")

    def _num_batches(self) -> int:
        """Number of way-id buckets to build the simplified graph in."""
        if self.batches and self.batches > 0:
            return self.batches
        n_refs = self.fetchone("SELECT COUNT(*) FROM way_nodes")[0] or 0
        return max(1, math.ceil(n_refs / NODE_REFS_PER_BATCH))

    def _use_global_junctions(self) -> bool:
        """True when the mode-agnostic ``main.global_junctions`` table exists (built by
        :class:`GlobalJunctions` as a pre-pass). When present, roads are segmented at that one shared
        junction set so a road keeps the same ``edge_id`` across driving/walking/cycling; when absent
        (flag off / clip build), fall back to the mode-local ``junctions`` table."""
        return bool(self.fetchone(
            "SELECT count(*) FROM information_schema.tables "
            "WHERE table_catalog = current_database() "
            "AND table_schema = 'main' AND table_name = 'global_junctions'")[0])

    def _find_junctions(self) -> None:
        """Identify junction nodes, and which of them are ROAD junctions.

        A node is a graph junction if it is shared by >1 way or is a way endpoint (kept as a routing
        node). It is a junction for ROADS (`is_road_junction`) when another way of this mode shares
        it (a road, or a footway / path / cycleway joining it mid-way) or a road ends there. With the
        shared ``main.global_junctions`` (the default) roads use that set instead, built from every
        OSM way, so a road is cut at the same points, and keeps the same edge_id, in every mode.
        """
        self.execute(f"""
            CREATE OR REPLACE TEMP TABLE node_counts AS
            SELECT node_id,
                   COUNT(*)                                                 AS way_count,
                   SUM(CASE WHEN is_endpoint THEN 1 ELSE 0 END)             AS endpoint_count,
                   SUM(CASE WHEN is_road THEN 1 ELSE 0 END)                 AS road_way_count,
                   SUM(CASE WHEN is_road AND is_endpoint THEN 1 ELSE 0 END) AS road_endpoint_count
            FROM (
                SELECT wn.node_id,
                       (wn.seq = 0 OR wn.seq = wn.max_seq) AS is_endpoint,
                       {_IS_ROAD}                          AS is_road
                FROM (
                    SELECT way_id, node_id, seq, MAX(seq) OVER (PARTITION BY way_id) AS max_seq
                    FROM way_nodes
                ) wn
                LEFT JOIN ways w ON w.osm_id = wn.way_id
            )
            GROUP BY node_id
        """)

        self.execute("""
            CREATE OR REPLACE TABLE junctions AS
            SELECT node_id,
                   (way_count > 1 OR road_endpoint_count > 0) AS is_road_junction
            FROM node_counts
            WHERE way_count > 1        -- shared between ways (kept as a routing node)
               OR endpoint_count > 0   -- endpoint of a way
        """)

        junction_count = self.fetchone("SELECT COUNT(*) FROM junctions")[0]
        road_junctions = self.fetchone("SELECT COUNT(*) FROM junctions WHERE is_road_junction")[0]
        logger.info(f"  Identified {junction_count:,} junction nodes ({road_junctions:,} road junctions)")

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
        # Roads split at ROAD junctions; paths/footways split at every junction. When the
        # mode-agnostic main.global_junctions set is present, roads use IT (so a road-class way that
        # another mode drops still marks the junction here — cross-mode edge_id alignment), and
        # paths/footways are cut there too: a crossing over a road this mode doesn't have (a
        # secondary without sidewalks, in walking) is cut where cycling cuts it, so it keeps the same
        # edge_ids. Otherwise fall back to this mode's local `junctions.is_road_junction`.
        if self._use_global_junctions():
            split_here = (f"CASE WHEN {_IS_ROAD} THEN (gj.node_id IS NOT NULL) "
                          f"ELSE (j.node_id IS NOT NULL OR gj.node_id IS NOT NULL) END")
            gj_join = "LEFT JOIN main.global_junctions gj ON wn.node_id = gj.node_id"
        else:
            split_here = (f"(j.node_id IS NOT NULL AND "
                          f"(NOT {_IS_ROAD} OR COALESCE(j.is_road_junction, FALSE)))")
            gj_join = ""
        self.execute(f"""
            CREATE OR REPLACE TABLE way_segments AS
            WITH nodes AS (
                SELECT
                    wn.way_id, wn.node_id, wn.seq,
                    {split_here} AS split_here
                FROM way_nodes wn
                LEFT JOIN ways w ON w.osm_id = wn.way_id
                LEFT JOIN junctions j ON wn.node_id = j.node_id
                {gj_join}
                WHERE {way_filter}
            ),
            marked AS (
                SELECT
                    way_id,
                    node_id,
                    seq,
                    split_here AS is_junction,
                    -- latest split at or before this node = start of its segment
                    max(CASE WHEN split_here THEN seq END) OVER (
                        PARTITION BY way_id ORDER BY seq
                        ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS seg_start_incl,
                    -- nearest split strictly before / after (segment links for junction nodes)
                    max(CASE WHEN split_here THEN seq END) OVER (
                        PARTITION BY way_id ORDER BY seq
                        ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING) AS prev_junction,
                    min(CASE WHEN split_here THEN seq END) OVER (
                        PARTITION BY way_id ORDER BY seq
                        ROWS BETWEEN 1 FOLLOWING AND UNBOUNDED FOLLOWING) AS next_junction
                FROM nodes
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
                w.access,
                w.junction,
                w.layer,
                w.bridge,
                w.tunnel,
                w.service,
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
                    access,
                    junction,
                    layer,
                    bridge,
                    tunnel,
                    service,
                    refs,
                    -- Drop repeated points so the cut below never sees a zero-length lead segment
                    -- (which makes ST_LineSubstring emit a `-nan -nan` vertex); the halves read this
                    -- cleaned `geometry`, and the midpoint is taken from the same cleaned line.
                    ST_RemoveRepeatedPoints(geometry) AS geometry,
                    length_m,
                    -- Deterministic virtual midpoint-node id (was -(edge_id), which depended on
                    -- the volatile row-number edge_id). Keyed by (osm_id, source, refs): the refs
                    -- salt keeps TWO loops of one way anchored at the SAME node distinct (matching
                    -- the arc-split vnid scheme, see _split_arcs_at_midpoint); a stable negative id
                    -- keeps the split edges reproducible across rebuilds.
                    -((hash(osm_id, source, refs::VARCHAR) >> 2)::BIGINT) AS virtual_node_id,
                    -- Midpoint = the 50%-by-length point, i.e. exactly where the two halves are
                    -- cut below (ST_LineSubstring at 0.5). Using the middle *vertex*
                    -- (ST_PointN at npoints/2) instead put the node a few metres off the cut, so
                    -- the half-edges' shared endpoint did not equal the virtual node's coord.
                    ST_LineInterpolatePoint(ST_RemoveRepeatedPoints(geometry), 0.5) AS midpoint_geom
                FROM simplified_edges_forward
                WHERE source = target
                  AND ST_NPoints(ST_RemoveRepeatedPoints(geometry)) >= 3
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
                access,
                junction,
                layer,
                bridge,
                tunnel,
                service,
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
                access,
                junction,
                layer,
                bridge,
                tunnel,
                service,
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
                   access,
                   junction, layer, bridge, tunnel, service, refs, geometry, is_reverse, length_m
            FROM simplified_edges_forward
            WHERE source != target
            UNION ALL
            SELECT edge_id, source, target, osm_id, highway, name, maxspeed, oneway, lanes, surface,
                   access,
                   junction, layer, bridge, tunnel, service, refs, ST_GeomFromText(geometry_text) AS geometry, is_reverse, length_m
            FROM split_loops
            WHERE geometry_text IS NOT NULL
              AND ST_NPoints(ST_GeomFromText(geometry_text)) >= 2
        """)
        self.execute("DROP TABLE simplified_edges_forward")
        self.execute("ALTER TABLE simplified_edges_forward_new RENAME TO simplified_edges_forward")

    def _split_antiparallel_pairs(self) -> None:
        """Split two-way antiparallel forward pairs with a virtual node so ``(osm_id, source,
        target)`` is unique among forward edges (and thus, after reverse twins, globally).

        A self-crossing / doubling-back way can produce two forward edges ``A->B`` and ``B->A`` for
        the same ``osm_id`` (different arcs of a lollipop; also the ``A->M`` / ``M->A`` halves that
        ``_split_self_loops`` emits). On a **two-way** road, once reverse twins are added the directed
        pair ``A->B`` then appears twice (the forward ``A->B`` arc + the reverse of the ``B->A`` arc) —
        only ``is_reverse`` tells them apart. Inserting a virtual node in the **longer** arc
        (geometry-preserving: the arc is split at its midpoint, not dropped) removes the antiparallel
        pair, so no directed node-pair repeats. One-way pairs never collide (no reverse twins) and are
        left alone. Runs on forward edges, after contraction, before reverse edges.
        """
        # The longer edge of each two-way antiparallel pair — exactly one per pair (tiebreak by
        # refs, i.e. arc content, so the choice is rebuild-stable; edge_id is a volatile row
        # number here). Real endpoints only; residual self-loops are handled by _split_self_loops.
        self.execute("""
            CREATE OR REPLACE TEMP TABLE _split_sel AS
            SELECT e1.edge_id
            FROM simplified_edges_forward e1
            JOIN simplified_edges_forward e2
              ON e1.osm_id = e2.osm_id AND e1.source = e2.target AND e1.target = e2.source
            WHERE NOT e1.oneway AND NOT e2.oneway
              AND e1.source <> e1.target
              AND (e1.length_m > e2.length_m
                   OR (e1.length_m = e2.length_m AND e1.refs::VARCHAR > e2.refs::VARCHAR))
        """)
        n = self._split_arcs_at_midpoint()
        if n:
            logger.info(f"  antiparallel split: {n:,} two-way antiparallel arc(s) split "
                        "with a virtual node")

    def _split_parallel_pairs(self) -> None:
        """Split same-direction parallel forward arcs with a virtual node so no two arcs of a way
        share the same directed ``(osm_id, source, target)``.

        A way that revisits the same junction pair in the same order (``A…B…A…B`` — a figure-8 /
        double lollipop) yields two forward arcs ``A->B``. Only one can survive a unique
        ``(osm_id, source, target)``; the re-key dedup used to silently DELETE the longer one
        (104.5 m of a Burnaby parking aisle, see the design doc). Instead, keep the shortest arc
        whole and split every other arc of the group at its midpoint — geometry-preserving, like
        the self-loop / antiparallel treatments. Applies regardless of ``oneway``: two forward
        ``A->B`` arcs collide with each other directly, no reverse twin involved. Runs after
        contraction, before the antiparallel split (which then only sees whole arcs).
        See docs/design/split_same_direction_parallels.md.
        """
        # Every arc that is longer than another arc with the SAME (osm_id, source, target) —
        # i.e. all but the shortest of each group (tiebreak by refs: content, rebuild-stable).
        self.execute("""
            CREATE OR REPLACE TEMP TABLE _split_sel AS
            SELECT e1.edge_id
            FROM simplified_edges_forward e1
            JOIN simplified_edges_forward e2
              ON e1.osm_id = e2.osm_id AND e1.source = e2.source AND e1.target = e2.target
             AND e1.edge_id <> e2.edge_id
            WHERE e1.source <> e1.target
              AND (e1.length_m > e2.length_m
                   OR (e1.length_m = e2.length_m AND e1.refs::VARCHAR > e2.refs::VARCHAR))
        """)
        n = self._split_arcs_at_midpoint()
        if n:
            logger.info(f"  parallel split: {n:,} same-direction parallel arc(s) split "
                        "with a virtual node")

    def _split_arcs_at_midpoint(self) -> int:
        """Split every forward arc listed in temp table ``_split_sel(edge_id)`` at its
        50%-by-length midpoint into two halves joined by a virtual node; returns the arc count.

        The virtual id is negative + deterministic: ``-(hash(osm_id, source, target,
        refs::VARCHAR) >> 2)``. Salting with ``refs`` (the arc's node list) makes it unique PER
        ARC — two parallel arcs sharing endpoints get distinct midpoints (endpoint-only hashing
        gave a figure-8's two arcs the same virtual id with two different geometries, a duplicate
        node_id). It also cannot collide with a self-loop virtual (same salt, different inputs).
        """
        if self.fetchone("SELECT COUNT(*) FROM _split_sel")[0] == 0:
            self.execute("DROP TABLE IF EXISTS _split_sel;")
            return 0

        self.execute("""
            CREATE OR REPLACE TEMP TABLE _ap_split AS
            WITH tosplit AS (
                SELECT e.* EXCLUDE (geometry),
                       -- ST_RemoveRepeatedPoints: an earlier substring cut that lands exactly on a
                       -- vertex leaves a zero-length lead segment (a duplicated point). Splitting
                       -- such an arc again makes DuckDB's ST_LineSubstring emit a `-nan -nan` vertex
                       -- (invalid geometry). Dropping repeated points first keeps the cut clean.
                       ST_RemoveRepeatedPoints(e.geometry) AS geometry,
                       -((hash(e.osm_id, e.source, e.target, e.refs::VARCHAR) >> 2)::BIGINT) AS vnid,
                       ST_LineInterpolatePoint(ST_RemoveRepeatedPoints(e.geometry), 0.5) AS mid
                FROM simplified_edges_forward e
                WHERE e.edge_id IN (SELECT edge_id FROM _split_sel)
                  AND ST_NPoints(ST_RemoveRepeatedPoints(e.geometry)) >= 2
            )
            SELECT source, vnid AS target, osm_id, highway, name, maxspeed, oneway, lanes,
                   surface, access, junction, layer, bridge, tunnel, service,
                   refs[1 : len(refs) / 2 + 1] AS refs,
                   ST_AsText(ST_LineSubstring(geometry, 0, 0.5)) AS gtext,
                   FALSE AS is_reverse, length_m / 2 AS length_m,
                   vnid, ST_AsText(mid) AS mtext
            FROM tosplit
            UNION ALL
            SELECT vnid AS source, target, osm_id, highway, name, maxspeed, oneway, lanes,
                   surface, access, junction, layer, bridge, tunnel, service,
                   refs[len(refs) / 2 + 1 : ] AS refs,
                   ST_AsText(ST_LineSubstring(geometry, 0.5, 1)) AS gtext,
                   FALSE AS is_reverse, length_m / 2 AS length_m,
                   vnid, ST_AsText(mid) AS mtext
            FROM tosplit
        """)

        # Rebuild forward edges: untouched edges + the split halves, with fresh row-number ids.
        self.execute("""
            CREATE OR REPLACE TABLE simplified_edges_forward AS
            SELECT (row_number() OVER ())::INTEGER AS edge_id, *
            FROM (
                SELECT source, target, osm_id, highway, name, maxspeed, oneway, lanes,
                       surface, access, junction, layer, bridge, tunnel, service, refs, geometry, is_reverse, length_m
                FROM simplified_edges_forward
                WHERE edge_id NOT IN (SELECT edge_id FROM _split_sel)
                UNION ALL
                SELECT source, target, osm_id, highway, name, maxspeed, oneway, lanes,
                       surface, access, junction, layer, bridge, tunnel, service, refs,
                       ST_GeomFromText(gtext) AS geometry, is_reverse, length_m
                FROM _ap_split
                WHERE gtext IS NOT NULL AND ST_NPoints(ST_GeomFromText(gtext)) >= 2
            )
        """)

        # Register the new virtual midpoint nodes (create the table if _split_self_loops didn't).
        self.execute("CREATE TABLE IF NOT EXISTS virtual_nodes (node_id BIGINT, geom GEOMETRY)")
        self.execute("""
            INSERT INTO virtual_nodes
            SELECT DISTINCT vnid, ST_GeomFromText(mtext) FROM _ap_split
        """)
        n = self.fetchone("SELECT COUNT(*) FROM _split_sel")[0]
        self.execute("DROP TABLE IF EXISTS _split_sel; DROP TABLE IF EXISTS _ap_split;")
        return n

    def _contract_chains(self) -> None:
        """Merge maximal chains of consecutive forward segments that are the SAME road.

        A node is a contraction point when it has exactly two forward edges that agree on
        EVERY carried attribute (highway, name, oneway, maxspeed, lanes, surface, junction),
        it is a real node (id > 0, so virtual self-loop midpoints stay intact), and — for
        ONE-WAY roads only — the two edges pass through it one-in/one-out. (Two-way forward
        edges carry an arbitrary per-way orientation, so that check is skipped for them.)
        This merges a street that OSM split into
        several way objects (different osm_id) — the common case — without blurring any
        attribute, since they are all required equal.

        Each maximal run collapses to one edge whose geometry/length/refs are rebuilt from
        the stitched node sequence, oriented along the road's travel direction: a one-way
        chain keeps its LEGAL direction (source -> target follows the OSM node order, so the
        single forward edge is drivable the right way), and a two-way chain is oriented
        deterministically by node id (its reverse edge is generated afterward, so either
        direction is valid). The merged edge keeps its FIRST member's osm_id (the source-end
        segment), so the stable edge_id hash (osm_id, source, target) stays
        well-defined. Runs on forward edges only, before reverse edges and the re-key.
        """
        # Each forward edge as two half-edges (at its source, at its target), carrying the
        # attributes the merge predicate compares.
        self.execute("""
            CREATE OR REPLACE TEMP TABLE _inc AS
            SELECT edge_id, source AS node, target AS other, refs, TRUE  AS fwd,
                   highway, name, oneway, maxspeed, lanes, surface, access, junction, layer, bridge, tunnel, service
            FROM simplified_edges_forward
            UNION ALL
            SELECT edge_id, target AS node, source AS other, refs, FALSE AS fwd,
                   highway, name, oneway, maxspeed, lanes, surface, access, junction, layer, bridge, tunnel, service
            FROM simplified_edges_forward
        """)
        # Contraction nodes: degree-2, real, and the two edges identical on every attribute
        # (chr(1) sentinel so NULL == NULL groups as one). The one-in/one-out check
        # (sum(fwd)=1) is required ONLY for one-way roads, where it guarantees a drivable
        # directed through-path. For two-way roads the forward-edge orientation is arbitrary
        # (each follows its own way's digitisation), so two consecutive segments can both point
        # out (sum=2) or both in (sum=0) at their shared node — still a valid degree-2 through
        # point (the reverse halves exist and the walk reverses refs per-step). Demanding
        # sum(fwd)=1 there wrongly skipped such chains; gate it on bool_or(oneway).
        # A global road junction is a HARD split that must survive contraction: even when a mode drops
        # the crossing road (so the node is locally degree-2), re-merging here would undo cross-mode
        # edge_id alignment. Exclude those nodes from contraction when the global set is present.
        gj_protect = ("AND node NOT IN (SELECT node_id FROM main.global_junctions)"
                      if self._use_global_junctions() else "")
        self.execute(f"""
            CREATE OR REPLACE TEMP TABLE _node2 AS
            SELECT node FROM _inc
            WHERE node > 0
            {gj_protect}
            GROUP BY node
            HAVING count(*) = 2 AND (NOT bool_or(oneway) OR sum(fwd::INT) = 1)
               AND count(DISTINCT highway) = 1
               AND count(DISTINCT coalesce(name, chr(1))) = 1
               AND count(DISTINCT oneway) = 1
               AND count(DISTINCT coalesce(maxspeed, chr(1))) = 1
               AND count(DISTINCT coalesce(CAST(lanes AS VARCHAR), chr(1))) = 1
               AND count(DISTINCT coalesce(surface, chr(1))) = 1
               -- a bus-only carriageway must never merge into a public one
               AND count(DISTINCT coalesce(access, chr(1))) = 1
               AND count(DISTINCT coalesce(junction, chr(1))) = 1
               AND count(DISTINCT coalesce(layer, chr(1))) = 1
               AND count(DISTINCT coalesce(bridge, chr(1))) = 1
               AND count(DISTINCT coalesce(tunnel, chr(1))) = 1
               AND count(DISTINCT coalesce(service, chr(1))) = 1
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
                       CASE WHEN i.fwd THEN i.refs ELSE list_reverse(i.refs) END AS refs_acc,
                       i.fwd AS seed_fwd, i.oneway AS oneway
                FROM _inc i
                WHERE i.node NOT IN (SELECT node FROM _node2)
                  AND i.other IN (SELECT node FROM _node2)
                UNION ALL
                SELECT w.start, i.edge_id, i.other,
                       list_append(w.edge_set, i.edge_id),
                       list_concat(w.refs_acc,
                           (CASE WHEN i.fwd THEN i.refs ELSE list_reverse(i.refs) END)[2:]),
                       w.seed_fwd, w.oneway
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
              AND start <> cur                           -- drop closed loops (same terminal)
              -- Orient the chain. One-way: keep the walk seeded in the legal travel
              -- direction (seed traversed forward), so source -> target is drivable. Two-way:
              -- either direction is valid (the reverse edge is added later), so dedupe the two
              -- mirror walks deterministically by node id.
              AND (CASE WHEN oneway THEN seed_fwd ELSE start < cur END)
        """)

        n_chains = self.fetchone("SELECT COUNT(*) FROM _chains")[0]
        if n_chains == 0:
            self._empty_edge_id_map()
            self.execute("DROP TABLE IF EXISTS _inc; DROP TABLE IF EXISTS _node2; "
                         "DROP TABLE IF EXISTS _chains;")
            logger.info("  merge_segments: no same-road chains to contract")
            return

        # Per-chain metadata: stitched endpoints/refs plus the uniform attributes and the
        # representative osm_id, all taken from the FIRST member (the source-end segment,
        # edge_set[1]). The members are identical on every carried attribute by the same-road
        # predicate, so any member would do for those; osm_id is the one field that differs,
        # and taking the first one keeps the stable edge_id hash well-defined.
        self.execute("""
            CREATE OR REPLACE TEMP TABLE _cmeta AS
            SELECT c.cid, c.source, c.target, c.refs_acc, e.osm_id,
                   e.highway, e.name, e.maxspeed, e.oneway, e.lanes, e.surface, e.access, e.junction,
                   e.layer, e.bridge, e.tunnel, e.service
            FROM _chains c
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
                a.lanes, a.surface, a.access, a.junction, a.layer, a.bridge, a.tunnel, a.service, a.refs_acc AS refs,
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
        # seq = the segment's position along the road (forward direction); osm_id = that
        # segment's way, so (ORDER BY seq) is the merged edge's constituent ways source->target
        # (seq 1 = source-end / first, MAX(seq) = target-end / last). Stable-hash ids on both
        # sides (same formula as _rekey_edges), forward + reverse (reverse only two-way).
        # Each segment is taken in the direction it runs along the chain (a two-way chain is
        # oriented by node id, and a segment can be drawn the other way), so old and new ids
        # always point the same way; is_reverse says whether that old edge runs against its way's
        # drawing. Edges NOT in this table are unchanged by the merge.
        self.execute("""
            CREATE OR REPLACE TABLE edge_id_map AS
            WITH mem AS (
                SELECT c.cid, m.seq, e.osm_id,
                       list_position(c.refs_acc, e.source) > list_position(c.refs_acc, e.target) AS against,
                       CASE WHEN list_position(c.refs_acc, e.source) < list_position(c.refs_acc, e.target)
                            THEN e.source ELSE e.target END AS a,      -- along the chain: a -> b
                       CASE WHEN list_position(c.refs_acc, e.source) < list_position(c.refs_acc, e.target)
                            THEN e.target ELSE e.source END AS b
                FROM _chains c, UNNEST(c.edge_set) WITH ORDINALITY AS m(eid, seq)
                JOIN simplified_edges_forward e ON e.edge_id = m.eid
            )
            SELECT (hash(mem.osm_id, mem.a, mem.b) >> 1)::BIGINT AS old_edge_id,
                   (hash(cm.osm_id, cm.source, cm.target) >> 1)::BIGINT AS new_edge_id,
                   mem.seq::INTEGER AS seq, mem.against AS is_reverse, mem.osm_id AS osm_id
            FROM mem JOIN _cmeta cm USING (cid)
            UNION ALL
            SELECT (hash(mem.osm_id, mem.b, mem.a) >> 1)::BIGINT,
                   (hash(cm.osm_id, cm.target, cm.source) >> 1)::BIGINT,
                   mem.seq::INTEGER, NOT mem.against, mem.osm_id
            FROM mem JOIN _cmeta cm USING (cid)
            WHERE NOT cm.oneway
        """)

        # Forward edges = untouched singletons + one edge per merged chain.
        self.execute("""
            CREATE OR REPLACE TABLE simplified_edges_forward AS
            SELECT (row_number() OVER ())::INTEGER AS edge_id, *
            FROM (
                SELECT source, target, osm_id, highway, name, maxspeed, oneway, lanes,
                       surface, access, junction, layer, bridge, tunnel, service, refs, geometry, is_reverse, length_m FROM _merged
                UNION ALL
                SELECT source, target, osm_id, highway, name, maxspeed, oneway, lanes,
                       surface, access, junction, layer, bridge, tunnel, service, refs, geometry, is_reverse, length_m
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
                old_edge_id BIGINT, new_edge_id BIGINT, seq INTEGER, is_reverse BOOLEAN,
                osm_id BIGINT)
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
                sef.access,
                sef.junction,
                sef.layer,
                sef.bridge,
                sef.tunnel,
                sef.service,
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
