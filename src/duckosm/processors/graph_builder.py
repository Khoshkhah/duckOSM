"""
Graph builder processor - builds edges from ways.
"""

from duckosm.processors.base import BaseProcessor


class GraphBuilder(BaseProcessor):
    """
    Build routing edges from OSM ways.
    
    Creates:
        - edges: Directed road segments with geometry and attributes
    
    Each way becomes one or more edges. For bidirectional roads,
    we create edges in both directions.
    """
    
    def run(self) -> None:
        """Build edge table."""
        self._build_edges()
        self._add_reverse_edges()
    
    def _build_edges(self) -> None:
        """Create forward edges from ways."""
        self.execute("""
            CREATE OR REPLACE TABLE edges AS
            WITH way_endpoints AS (
                SELECT 
                    w.osm_id,
                    w.highway,
                    w.name,
                    w.maxspeed,
                    w.oneway,
                    w.lanes,
                    w.surface,
                    w.refs[1] AS source,
                    w.refs[len(w.refs)] AS target,
                    len(w.refs) AS node_count
                FROM ways w
            )
            SELECT 
                row_number() OVER () AS edge_id,
                source,
                target,
                osm_id,
                highway,
                name,
                maxspeed,
                oneway,
                lanes,
                surface,
                node_count,
                -- Haversine distance in meters
                CAST(
                    12742000 * ASIN(SQRT(
                        POWER(SIN(RADIANS(ST_Y(n2.geom) - ST_Y(n1.geom)) / 2), 2) +
                        COS(RADIANS(ST_Y(n1.geom))) * COS(RADIANS(ST_Y(n2.geom))) *
                        POWER(SIN(RADIANS(ST_X(n2.geom) - ST_X(n1.geom)) / 2), 2)
                    ))
                AS DOUBLE) AS length_m,
                -- Create geometry as native GEOMETRY type in [Lon, Lat] order
                ST_GeomFromText('LINESTRING(' || ST_X(n1.geom) || ' ' || ST_Y(n1.geom) || ', ' ||
                                                 ST_X(n2.geom) || ' ' || ST_Y(n2.geom) || ')') AS geometry,
                FALSE AS is_reverse
            FROM way_endpoints we
            LEFT JOIN nodes n1 ON n1.node_id = we.source
            LEFT JOIN nodes n2 ON n2.node_id = we.target
            WHERE n1.geom IS NOT NULL AND n2.geom IS NOT NULL
            AND source != target  -- Remove self-loops
        """)
    
    def _add_reverse_edges(self) -> None:
        """Add reverse edges for bidirectional roads."""
        # Get max edge_id
        max_id = self.fetchone("SELECT MAX(edge_id) FROM edges")[0] or 0
        
        self.execute(f"""
            INSERT INTO edges
            SELECT 
                {max_id} + row_number() OVER () AS edge_id,
                target AS source,
                source AS target,
                osm_id,
                highway,
                name,
                maxspeed,
                oneway,
                lanes,
                surface,
                node_count,
                length_m,
                -- Reverse geometry natively
                ST_Reverse(geometry) AS geometry,
                TRUE AS is_reverse
            FROM edges
            WHERE oneway IS NULL 
               OR oneway NOT IN ('yes', '1', 'true', '-1')
        """)
