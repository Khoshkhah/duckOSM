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
                    w.lanes_fwd,
                    w.surface,
                    w.access,
                    w.junction,
                    w.refs[1] AS source,
                    w.refs[len(w.refs)] AS target,
                    len(w.refs) AS node_count
                FROM ways w
            )
            SELECT
                -- Stable, deterministic edge_id: a hash of the edge's identity (so rebuilds
                -- don't renumber the graph). See GraphSimplifier._rekey_edges for rationale.
                (hash(osm_id, source, target) >> 1)::BIGINT AS edge_id,
                source,
                target,
                osm_id,
                highway,
                name,
                maxspeed,
                oneway,
                lanes_fwd AS lanes,
                surface,
                access,
                junction,
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
                -- Reverse edge: hash its OWN (swapped) endpoints, so it gets a stable id distinct
                -- from the forward edge (source->target already encodes direction; no is_reverse).
                (hash(e.osm_id, e.target, e.source) >> 1)::BIGINT AS edge_id,
                e.target AS source,
                e.source AS target,
                e.osm_id,
                e.highway,
                e.name,
                e.maxspeed,
                e.oneway,
                -- Reverse edge carries the backward lane count.
                w.lanes_bwd AS lanes,
                e.surface,
                e.access,
                e.junction,
                e.node_count,
                e.length_m,
                -- Reverse geometry natively
                ST_Reverse(e.geometry) AS geometry,
                TRUE AS is_reverse
            FROM edges e
            JOIN ways w ON w.osm_id = e.osm_id
            -- Two-way roads get a reverse edge. oneway is the single source of truth
            -- (roundabouts were already normalised to oneway=TRUE upstream).
            WHERE NOT e.oneway
        """)
