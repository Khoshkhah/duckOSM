"""
Restriction processor - extracts and maps turn restrictions.
"""

from duckosm.processors.base import BaseProcessor


class RestrictionProcessor(BaseProcessor):
    """
    Extract turn restrictions from OSM relations.
    
    Creates:
        - turn_restrictions: Restriction rules mapped to edge IDs
    """
    
    def run(self) -> None:
        """Extract and process restrictions."""
        self._extract_raw_restrictions()
        self._map_to_edges()
    
    def _extract_raw_restrictions(self) -> None:
        """Extract restriction relations from raw OSM data."""
        # Use map_extract for reliable tag access in DuckDB
        self.execute("""
            CREATE OR REPLACE TABLE restrictions_raw AS
            SELECT 
                osm_id AS restriction_id,
                map_extract(tags, 'restriction')[1] AS restriction_type,
                refs,
                ref_roles,
                ref_types
            FROM raw.relations
            WHERE map_extract(tags, 'type')[1] = 'restriction'
            AND map_extract(tags, 'restriction')[1] IS NOT NULL
        """)
    
    def _map_to_edges(self) -> None:
        """Map restrictions to edge IDs."""
        # Unnest the parallel arrays to get from/via/to
        self.execute("""
            CREATE OR REPLACE TABLE restrictions_unnested AS
            SELECT 
                restriction_id,
                restriction_type,
                UNNEST(refs) AS ref_id,
                UNNEST(ref_roles) AS role,
                UNNEST(ref_types) AS ref_type
            FROM restrictions_raw
        """)
        
        # Pivot to get from_way, via_node, to_way
        self.execute("""
            CREATE OR REPLACE TABLE restrictions_pivoted AS
            SELECT 
                restriction_id,
                restriction_type,
                MAX(CASE WHEN role = 'from' AND ref_type = 'way' THEN ref_id END) AS from_way,
                MAX(CASE WHEN role = 'via' AND ref_type = 'node' THEN ref_id END) AS via_node,
                MAX(CASE WHEN role = 'to' AND ref_type = 'way' THEN ref_id END) AS to_way
            FROM restrictions_unnested
            GROUP BY restriction_id, restriction_type
        """)
        
        # Map to edge IDs
        self.execute("""
            CREATE OR REPLACE TABLE turn_restrictions AS
            SELECT 
                r.restriction_id,
                r.restriction_type,
                r.via_node,
                e1.edge_id AS from_edge_id,
                e2.edge_id AS to_edge_id
            FROM restrictions_pivoted r
            -- From edge: ends at via_node
            LEFT JOIN edges e1 ON e1.osm_id = r.from_way AND e1.target = r.via_node
            -- To edge: starts at via_node
            LEFT JOIN edges e2 ON e2.osm_id = r.to_way AND e2.source = r.via_node
            WHERE e1.edge_id IS NOT NULL AND e2.edge_id IS NOT NULL
        """)
        
        # Cleanup temp tables
        self.execute("DROP TABLE IF EXISTS restrictions_raw")
        self.execute("DROP TABLE IF EXISTS restrictions_unnested")
        self.execute("DROP TABLE IF EXISTS restrictions_pivoted")
