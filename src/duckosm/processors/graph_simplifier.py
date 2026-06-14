"""
Graph simplifier processor - contracts degree-2 nodes and splits ways at junctions.
"""

import time
import logging
from duckosm.processors.base import BaseProcessor

logger = logging.getLogger(__name__)

class GraphSimplifier(BaseProcessor):
    """
    Simplifies the road network by contracting degree-2 nodes.
    
    This results in a graph where:
    1. Every node is a junction (degree != 2) or an endpoint.
    2. Edges contain the full geometry (stitching intermediate points).
    3. Length and costs are correctly aggregated.
    """
    
    def run(self) -> None:
        """Run the simplification pipeline."""
        logger.info("Simplifying graph...")
        start = time.time()
        
        # 1. Identify junction nodes
        self._find_junctions()
        
        # 2. Segment ways at junctions
        self._segment_ways()
        
        # 3. Build simplified edges with full geometry
        self._build_simplified_edges()
        
        # 4. Split self-loops with virtual midpoint nodes
        self._split_self_loops()
        
        # 5. Finalize tables
        self._finalize_tables()
        
        elapsed = time.time() - start
        logger.info(f"  Graph simplified in {elapsed:.2f}s")

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

    def _segment_ways(self) -> None:
        """Split ways into segments between junctions, ensuring junctions are shared.

        A segment runs from one junction to the next; junction nodes are shared between
        the two adjacent segments (they are the end of one and the start of the next).
        ``segment_idx`` is the sequence index of the segment's starting junction.

        This is computed with O(N) window functions over the (already ordered) way
        nodes. The earlier implementation joined nodes to segment ranges with an
        inequality (``seq BETWEEN start AND end``), which materialised N*K rows per way
        (N nodes x K segments) and OOM'd on dense, highly-connected networks (cycling).
        """
        self.execute("""
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

    def _build_simplified_edges(self) -> None:
        """Create simplified edges by stitching node geometries."""
        # Stitch into LineStrings and calculate length
        self.execute("""
            CREATE OR REPLACE TABLE simplified_edges_forward AS
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
                row_number() OVER ()::INTEGER AS edge_id,
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
                    -(edge_id) AS virtual_node_id,
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
