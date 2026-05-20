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
        """Split ways into segments between junctions, ensuring junctions are shared."""
        # 1. Identify all junction points for each way
        self.execute("""
            CREATE OR REPLACE TEMP TABLE way_junction_seqs AS
            SELECT way_id, seq
            FROM way_nodes wn
            JOIN junctions j ON wn.node_id = j.node_id
            ORDER BY way_id, seq
        """)
        
        # 2. Pair consecutive junctions into segments
        self.execute("""
            CREATE OR REPLACE TEMP TABLE way_segment_ranges AS
            SELECT 
                way_id,
                seq as start_seq,
                LEAD(seq) OVER (PARTITION BY way_id ORDER BY seq) as end_seq
            FROM way_junction_seqs
        """)
        
        # 3. Create the way_segments table by joining back to way_nodes
        self.execute("""
            CREATE OR REPLACE TABLE way_segments AS
            SELECT 
                r.way_id,
                r.start_seq as segment_idx,
                wn.node_id,
                wn.seq
            FROM way_nodes wn
            JOIN way_segment_ranges r ON wn.way_id = r.way_id 
                AND wn.seq >= r.start_seq 
                AND wn.seq <= r.end_seq
            WHERE r.end_seq IS NOT NULL
        """)

    def _build_simplified_edges(self) -> None:
        """Create simplified edges by stitching node geometries."""
        # Get coordinates for all nodes in segments
        self.execute("""
            CREATE OR REPLACE TEMP TABLE segment_geometry_prep AS
            SELECT 
                ws.way_id,
                ws.segment_idx,
                ws.node_id,
                ws.seq,
                n.lat,
                n.lon
            FROM way_segments ws
            JOIN raw.nodes n ON ws.node_id = n.osm_id
            ORDER BY ws.way_id, ws.segment_idx, ws.seq
        """)
        
        # Stitch into LineStrings and calculate length
        self.execute("""
            CREATE OR REPLACE TABLE simplified_edges_forward AS
            WITH segment_groups AS (
                SELECT 
                    ws.way_id,
                    ws.segment_idx,
                    LIST(ws.node_id ORDER BY ws.seq) as node_list,
                    LIST(n.lon || ' ' || n.lat ORDER BY ws.seq) as coord_list,
                    map_extract(w.tags, 'highway')[1] as highway,
                    map_extract(w.tags, 'name')[1] as name,
                    map_extract(w.tags, 'maxspeed')[1] as maxspeed,
                    map_extract(w.tags, 'oneway')[1] as oneway,
                    map_extract(w.tags, 'lanes')[1] as lanes,
                    map_extract(w.tags, 'surface')[1] as surface,
                    map_extract(w.tags, 'access')[1] as access,
                    w.tags
                FROM way_segments ws
                JOIN raw.nodes n ON ws.node_id = n.osm_id
                JOIN ways w ON ws.way_id = w.osm_id
                GROUP BY ws.way_id, ws.segment_idx, w.tags
            )
            SELECT 
                row_number() OVER ()::INTEGER AS edge_id,
                node_list[1] AS source,
                node_list[len(node_list)] AS target,
                way_id AS osm_id,
                highway,
                name,
                maxspeed,
                oneway,
                lanes,
                surface,
                node_list as refs,
                ST_GeomFromText('LINESTRING(' || list_aggregate(coord_list, 'string_agg', ', ') || ')') AS geometry,
                FALSE AS is_reverse
            FROM segment_groups
            WHERE len(coord_list) >= 2
        """)

        # Now calculate actual lengths using Haversine formula
        # ST_Length_Spheroid is broken in DuckDB spatial for certain coordinate ranges
        self.execute("ALTER TABLE simplified_edges_forward ADD COLUMN length_m FLOAT")

        self.execute("""
            WITH edge_npoints AS (
                SELECT edge_id, geometry, ST_NPoints(geometry) as npoints
                FROM simplified_edges_forward
            ),
            point_indices AS (
                SELECT edge_id, geometry, UNNEST(generate_series(1, npoints::INTEGER))::INTEGER as idx
                FROM edge_npoints
            ),
            edge_points AS (
                SELECT 
                    edge_id, 
                    idx,
                    ST_X(ST_PointN(geometry, idx)) as lon,
                    ST_Y(ST_PointN(geometry, idx)) as lat
                FROM point_indices
            ),
            segment_lengths AS (
                SELECT 
                    p1.edge_id,
                    12742000 * ASIN(SQRT(
                        POWER(SIN(RADIANS(p2.lat - p1.lat) / 2), 2) +
                        COS(RADIANS(p1.lat)) * COS(RADIANS(p2.lat)) *
                        POWER(SIN(RADIANS(p2.lon - p1.lon) / 2), 2)
                    )) as segment_m
                FROM edge_points p1
                JOIN edge_points p2 ON p1.edge_id = p2.edge_id AND p2.idx = p1.idx + 1
            ),
            total_lengths AS (
                SELECT edge_id, SUM(segment_m) as total_m
                FROM segment_lengths
                GROUP BY edge_id
            )
            UPDATE simplified_edges_forward 
            SET length_m = total_lengths.total_m
            FROM total_lengths
            WHERE simplified_edges_forward.edge_id = total_lengths.edge_id;
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
                   refs, geometry, is_reverse, length_m
            FROM simplified_edges_forward
            WHERE source != target
            UNION ALL
            SELECT edge_id, source, target, osm_id, highway, name, maxspeed, oneway, lanes, surface,
                   refs, ST_GeomFromText(geometry_text) AS geometry, is_reverse, length_m
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
                target AS source,
                source AS target,
                osm_id,
                highway,
                name,
                maxspeed,
                oneway,
                lanes,
                surface,
                list_reverse(refs) as refs,
                -- Native reverse geometry
                ST_Reverse(geometry),
                TRUE AS is_reverse,
                length_m
            FROM simplified_edges_forward
            WHERE oneway IS NULL OR oneway NOT IN ('yes', '1', 'true', '-1')
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
