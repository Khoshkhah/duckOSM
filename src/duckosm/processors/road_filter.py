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
                (
                    (
                        map_extract(tags, 'highway')[1] IN (
                            'cycleway', 'path', 'track', 'bridleway', 'living_street',
                            'residential', 'service', 'unclassified',
                            'tertiary', 'tertiary_link', 'secondary', 'secondary_link',
                            'primary', 'primary_link'
                        )
                        OR map_extract(tags, 'bicycle')[1] IN ('yes', 'designated', 'permissive')
                        -- A road carrying explicit cycle infrastructure, but not
                        -- 'no'/'none'/'separate' (the latter means use the separately
                        -- mapped cycleway, not this road).
                        OR (
                            map_extract(tags, 'cycleway')[1] IS NOT NULL
                            AND map_extract(tags, 'cycleway')[1] NOT IN ('no', 'none', 'separate')
                        )
                    )
                    -- Drop ways where cycling is explicitly forbidden ...
                    AND COALESCE(map_extract(tags, 'bicycle')[1], '') <> 'no'
                    -- ... or that are private/closed, unless bicycles are explicitly allowed.
                    AND (
                        COALESCE(map_extract(tags, 'access')[1], '') NOT IN ('private', 'no')
                        OR map_extract(tags, 'bicycle')[1] IN ('yes', 'designated', 'permissive')
                    )
                )
            """
        else:
            raise ValueError(f"Unknown mode: {self.mode}")

        oneway_expr = self._oneway_expression()
            
        self.execute(f"""
            CREATE OR REPLACE TABLE ways AS
            WITH base AS (
                SELECT
                    osm_id,
                    map_extract(tags, 'highway')[1] AS highway,
                    map_extract(tags, 'name')[1] AS name,
                    map_extract(tags, 'maxspeed')[1] AS maxspeed,
                    -- One-way flag as a boolean (mode-aware; see _oneway_expression).
                    -- Never NULL: two-way / untagged -> FALSE.
                    {oneway_expr} AS oneway,
                    map_extract(tags, 'surface')[1] AS surface,
                    map_extract(tags, 'access')[1] AS access,
                    map_extract(tags, 'junction')[1] AS junction,
                    -- Parsed integer lane counts (first integer in the tag; NULL when
                    -- untagged, zero or non-numeric).
                    NULLIF(TRY_CAST(regexp_extract(map_extract(tags, 'lanes')[1], '\\d+') AS INTEGER), 0) AS n_total,
                    NULLIF(TRY_CAST(regexp_extract(map_extract(tags, 'lanes:forward')[1], '\\d+') AS INTEGER), 0) AS n_fwd,
                    NULLIF(TRY_CAST(regexp_extract(map_extract(tags, 'lanes:backward')[1], '\\d+') AS INTEGER), 0) AS n_bwd,
                    tags,
                    refs
                FROM raw.ways
                WHERE {where_clause}
            )
            SELECT
                osm_id,
                highway,
                name,
                maxspeed,
                oneway,
                surface,
                access,
                junction,
                -- Lane count in the forward direction. Prefer lanes:forward; for one-way
                -- roads all lanes are forward; otherwise split the two-way total (forward
                -- gets the larger half). Fall back to a class default when nothing is tagged.
                CASE
                    WHEN n_fwd IS NOT NULL THEN n_fwd
                    WHEN oneway
                        THEN COALESCE(n_total, CASE WHEN highway IN ('motorway', 'trunk') THEN 2 ELSE 1 END)
                    WHEN n_total IS NOT NULL AND n_bwd IS NOT NULL THEN GREATEST(n_total - n_bwd, 1)
                    WHEN n_total IS NOT NULL THEN GREATEST(CAST(CEIL(n_total / 2.0) AS INTEGER), 1)
                    ELSE CASE WHEN highway IN ('motorway', 'trunk') THEN 2 ELSE 1 END
                END AS lanes_fwd,
                -- Lane count in the backward direction (used by the reverse edge of two-way
                -- roads). Prefer lanes:backward, else the remaining/half of the total, else
                -- the class default.
                CASE
                    WHEN n_bwd IS NOT NULL THEN n_bwd
                    WHEN n_total IS NOT NULL AND n_fwd IS NOT NULL THEN GREATEST(n_total - n_fwd, 1)
                    WHEN n_total IS NOT NULL THEN GREATEST(CAST(FLOOR(n_total / 2.0) AS INTEGER), 1)
                    ELSE CASE WHEN highway IN ('motorway', 'trunk') THEN 2 ELSE 1 END
                END AS lanes_bwd,
                tags,
                refs
            FROM base
        """)

    def _oneway_expression(self) -> str:
        """SQL boolean expression for the one-way flag, tailored to the mode.

        Roundabouts and OSM ``oneway`` in (yes/1/true/-1) are one-way; ``-1`` means
        one-way against the digitisation direction. For cycling, ``oneway:bicycle``
        overrides the generic ``oneway`` so contraflow cycling on one-way streets
        (very common in Europe) yields a reverse edge.
        """
        if self.mode == "cycling":
            return """
                CASE
                    WHEN map_extract(tags, 'oneway:bicycle')[1] = 'no' THEN FALSE
                    WHEN map_extract(tags, 'oneway:bicycle')[1] IN ('yes', '1', 'true', '-1') THEN TRUE
                    WHEN map_extract(tags, 'junction')[1] IN ('roundabout', 'circular') THEN TRUE
                    WHEN map_extract(tags, 'oneway')[1] IN ('yes', '1', 'true', '-1') THEN TRUE
                    ELSE FALSE
                END
            """
        return """
            CASE
                WHEN map_extract(tags, 'junction')[1] IN ('roundabout', 'circular') THEN TRUE
                WHEN map_extract(tags, 'oneway')[1] IN ('yes', '1', 'true', '-1') THEN TRUE
                ELSE FALSE
            END
        """

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
