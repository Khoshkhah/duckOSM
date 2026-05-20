"""
Road filter processor - extracts highway features from raw OSM data.
"""

from duckosm.processors.base import BaseProcessor


class RoadFilter(BaseProcessor):
    """
    Filter OSM data to highway features only.
    
    Creates:
        - nodes: All nodes from highway ways
        - ways: Highway ways with tags
        - way_nodes: Way-node relationships
    """
    
    def __init__(self, con, mode: str = "driving"):
        super().__init__(con)
        self.mode = mode
    
    def run(self) -> None:
        """Extract highway features."""
        self._create_ways_table()
        self._create_way_nodes_table()
        self._create_nodes_table()
    
    def _create_ways_table(self) -> None:
        """Create ways table with highway features."""
        if self.mode == "driving":
            exclude_highways = [
                'footway', 'cycleway', 'path', 'pedestrian',
                'steps', 'corridor', 'bridleway', 'construction',
                'proposed', 'raceway', 'bus_guideway', 'escape',
                'platform', 'elevator', 'track'
            ]
            exclude_list = ", ".join(f"'{h}'" for h in exclude_highways)
            where_clause = f"""
                map_extract(tags, 'highway')[1] IS NOT NULL
                AND map_extract(tags, 'highway')[1] NOT IN ({exclude_list})
            """
        elif self.mode == "walking":
            where_clause = """
                map_extract(tags, 'highway')[1] IN (
                    'footway', 'path', 'pedestrian', 'steps', 'living_street', 
                    'residential', 'service', 'platform', 'corridor'
                )
                OR map_extract(tags, 'sidewalk')[1] IN ('yes', 'both', 'left', 'right')
                OR map_extract(tags, 'foot')[1] IN ('yes', 'designated')
            """
        elif self.mode == "cycling":
            where_clause = """
                map_extract(tags, 'highway')[1] IN (
                    'cycleway', 'path', 'track', 'bridleway', 'living_street',
                    'residential', 'service', 'unclassified', 'tertiary', 'secondary'
                )
                OR map_extract(tags, 'bicycle')[1] IN ('yes', 'designated')
                OR map_extract(tags, 'cycleway')[1] IS NOT NULL
            """
        else:
            raise ValueError(f"Unknown mode: {self.mode}")
            
        self.execute(f"""
            CREATE OR REPLACE TABLE ways AS
            SELECT 
                osm_id,
                map_extract(tags, 'highway')[1] AS highway,
                map_extract(tags, 'name')[1] AS name,
                map_extract(tags, 'maxspeed')[1] AS maxspeed,
                map_extract(tags, 'oneway')[1] AS oneway,
                map_extract(tags, 'lanes')[1] AS lanes,
                map_extract(tags, 'surface')[1] AS surface,
                map_extract(tags, 'access')[1] AS access,
                tags,
                refs
            FROM raw.ways
            WHERE {where_clause}
        """)
    
    def _create_way_nodes_table(self) -> None:
        """Create way_nodes junction table."""
        self.execute("""
            CREATE OR REPLACE TABLE way_nodes AS
            SELECT 
                osm_id AS way_id,
                UNNEST(refs) AS node_id,
                UNNEST(range(len(refs))) AS seq
            FROM ways
        """)
    
    def _create_nodes_table(self) -> None:
        """Create nodes table with coordinates."""
        self.execute("""
            CREATE OR REPLACE TABLE nodes AS
            SELECT DISTINCT
                rn.osm_id AS node_id,
                ST_Point(rn.lon, rn.lat) AS geom
            FROM raw.nodes rn
            INNER JOIN way_nodes wn ON rn.osm_id = wn.node_id
        """)
