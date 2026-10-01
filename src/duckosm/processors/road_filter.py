"""
Road filter processor - extracts highway features from raw OSM data.
"""

from duckosm.processors.base import BaseProcessor


# Access (docs/design/access_private.md): per mode, the most specific access tag decides. A value
# in FORBIDDEN keeps the way out of that mode; 'private' keeps it, but the build moves it to
# <mode>.private_edges (visible on maps, never routable). In driving, a road only buses (or taxis,
# `psv`) may use is kept as 'bus' and moved there too (docs/design/bus_only_edges.md).
FORBIDDEN = {"driving": "'no', 'agricultural', 'forestry', 'emergency'",
             "walking": "'no'", "cycling": "'no'"}


def _tag(k):
    return f"map_extract(tags, '{k}')[1]"


def _most_specific(*keys):
    return f"COALESCE({', '.join(_tag(k) for k in keys)})"


class RoadFilter(BaseProcessor):
    """
    Filter OSM data to highway features only.
    
    Creates:
        - nodes: All nodes from highway ways
        - ways: Highway ways with tags
        - way_nodes: Way-node relationships
    """
    
    def __init__(self, con, mode: str = "driving", cycling_dismount: bool = True):
        super().__init__(con)
        self.mode = mode
        self.cycling_dismount = cycling_dismount
    
    def run(self) -> None:
        """Extract highway features."""
        self._create_ways_table()
        self._create_way_nodes_table()
        self._create_nodes_table()
    
    def _create_ways_table(self) -> None:
        """Create ways table with highway features."""
        # Default: keep the OSM highway tag verbatim. Driving mode overrides this to reclass
        # "rescued" shared streets (drivable pedestrian/footway ways) so downstream gets the
        # right capacity. The original class is always preserved in the `tags` map.
        highway_expr = "map_extract(tags, 'highway')[1]"
        if self.mode == "driving":
            # Class-trusting driving filter. Road classes are kept UNCONDITIONALLY (including
            # the _link ramps, service and living_street — omitting those silently disconnects
            # every interchange and shared street). A highway=pedestrian way is rescued ONLY
            # when it explicitly admits cars (motorcar/motor_vehicle in yes/designated/
            # permissive) or carries a drivable-restricted access tag (delivery/destination/
            # agricultural) — a shared street that genuinely carries cars; it is reclassed to
            # living_street so downstream capacity/speed treat it as the slow drivable street
            # it is. The original highway tag stays available in `tags`.
            road_classes = [
                'motorway', 'motorway_link', 'trunk', 'trunk_link',
                'primary', 'primary_link', 'secondary', 'secondary_link',
                'tertiary', 'tertiary_link', 'unclassified', 'residential',
                'service', 'living_street', 'road', 'busway',      # busway: bus-only, see _access_expression
            ]
            road_list = ", ".join(f"'{h}'" for h in road_classes)
            mv = ("COALESCE(map_extract(tags, 'motorcar')[1], "
                  "map_extract(tags, 'motor_vehicle')[1])")
            ped_ok = (
                "map_extract(tags, 'highway')[1] = 'pedestrian' AND ("
                f"{mv} IN ('yes', 'designated', 'permissive') "
                "OR map_extract(tags, 'access')[1] IN ('delivery', 'destination', 'agricultural'))"
            )
            highway_expr = f"""
                CASE WHEN {ped_ok} THEN 'living_street'
                     ELSE map_extract(tags, 'highway')[1] END
            """
            where_clause = f"""
                map_extract(tags, 'highway')[1] IN ({road_list})
                OR ({ped_ok})
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
                )
            """
            if self.cycling_dismount:
                # Dismount (push-the-bike) ways: footway/pedestrian enter the cycling graph even
                # with bicycle=no — pushing is walking, so their access is the walking one (below).
                # See docs/design/cycling_dismount_edges.md.
                where_clause = f"""
                    ({where_clause})
                    OR map_extract(tags, 'highway')[1] IN ('footway', 'pedestrian')
                """
        else:
            raise ValueError(f"Unknown mode: {self.mode}")

        direction_expr = self._direction_expression()
        access_expr = self._access_expression()
        bus_back_expr = (f"COALESCE({_tag('oneway:bus')} = 'no' OR {_tag('oneway:psv')} = 'no', FALSE)"
                         if self.mode == "driving" else "FALSE")
        where_clause = f"({where_clause}) AND COALESCE({access_expr}, '') NOT IN ({FORBIDDEN[self.mode]})"
            
        self.execute(f"""
            CREATE OR REPLACE TABLE ways AS
            WITH base AS (
                SELECT
                    osm_id,
                    {highway_expr} AS highway,
                    map_extract(tags, 'name')[1] AS name,
                    map_extract(tags, 'maxspeed')[1] AS maxspeed,
                    -- Travel direction (mode-aware; see _direction_expression): 1 one-way as
                    -- drawn, -1 one-way against the drawing (OSM oneway=-1), 0 two-way.
                    {direction_expr} AS dir,
                    -- driving: buses may also go against this one-way way (a contraflow bus lane)
                    {bus_back_expr} AS bus_contra,
                    map_extract(tags, 'surface')[1] AS surface,
                    -- service subtag (driveway/parking_aisle/alley/...) — drives the
                    -- narrow-vs-wide service-road rendering, like openstreetmap-carto.
                    map_extract(tags, 'service')[1] AS service,
                    -- the mode's effective access: its most specific access tag (_access_expression);
                    -- 'private' ways are moved to private_edges later in the build
                    {access_expr} AS access,
                    map_extract(tags, 'junction')[1] AS junction,
                    -- Vertical layering: layer (signed int as string), bridge, tunnel — for
                    -- draw-order (z_order += 10*layer) and bridge/tunnel styling downstream.
                    map_extract(tags, 'layer')[1] AS layer,
                    map_extract(tags, 'bridge')[1] AS bridge,
                    map_extract(tags, 'tunnel')[1] AS tunnel,
                    -- Parsed integer lane counts (first integer in the tag; NULL when
                    -- untagged, zero or non-numeric).
                    NULLIF(TRY_CAST(regexp_extract(map_extract(tags, 'lanes')[1], '\\d+') AS INTEGER), 0) AS n_total,
                    NULLIF(TRY_CAST(regexp_extract(map_extract(tags, 'lanes:forward')[1], '\\d+') AS INTEGER), 0) AS n_fwd,
                    NULLIF(TRY_CAST(regexp_extract(map_extract(tags, 'lanes:backward')[1], '\\d+') AS INTEGER), 0) AS n_bwd,
                    NULLIF(TRY_CAST(regexp_extract(map_extract(tags, 'lanes:reversible')[1], '\\d+') AS INTEGER), 0) AS n_rev,
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
                -- One-way flag as a boolean. Never NULL: two-way / untagged -> FALSE.
                dir <> 0 AS oneway,
                -- a one-way way whose reverse is a bus lane: the build adds that reverse edge as
                -- access 'bus' (moved to private_edges, drawn but never routed)
                dir <> 0 AND bus_contra AS bus_back,
                surface,
                service,
                access,
                junction,
                layer,
                bridge,
                tunnel,
                -- Lane count in the forward direction. Prefer lanes:forward; for one-way
                -- roads all lanes are forward; otherwise split the two-way total (forward
                -- gets the larger half). Fall back to a class default when nothing is tagged.
                -- A `lanes:reversible` (counterflow) lane is shared and available to each
                -- direction at its peak, so it's added to both directions' capacity — e.g.
                -- Lions Gate Bridge (lanes=3, forward=1, backward=1, reversible=1) -> 2 each way.
                -- COALESCE(n_rev, 0) is a no-op for the vast majority of roads (no reversible tag).
                CASE
                    -- one-way against the drawing: its lanes are the backward ones (refs reversed below)
                    WHEN dir = -1 THEN COALESCE(n_bwd, n_total, CASE WHEN highway IN ('motorway', 'trunk') THEN 2 ELSE 1 END)
                    WHEN n_fwd IS NOT NULL THEN n_fwd
                    WHEN dir <> 0
                        THEN COALESCE(n_total, CASE WHEN highway IN ('motorway', 'trunk') THEN 2 ELSE 1 END)
                    WHEN n_total IS NOT NULL AND n_bwd IS NOT NULL THEN GREATEST(n_total - n_bwd, 1)
                    WHEN n_total IS NOT NULL THEN GREATEST(CAST(CEIL(n_total / 2.0) AS INTEGER), 1)
                    ELSE CASE WHEN highway IN ('motorway', 'trunk') THEN 2 ELSE 1 END
                END + COALESCE(n_rev, 0) AS lanes_fwd,
                -- Lane count in the backward direction (used by the reverse edge of two-way
                -- roads). Prefer lanes:backward, else the remaining/half of the total, else
                -- the class default. A shared reversible lane is added here too (see above).
                CASE
                    WHEN n_bwd IS NOT NULL THEN n_bwd
                    WHEN n_total IS NOT NULL AND n_fwd IS NOT NULL THEN GREATEST(n_total - n_fwd, 1)
                    WHEN n_total IS NOT NULL THEN GREATEST(CAST(FLOOR(n_total / 2.0) AS INTEGER), 1)
                    ELSE CASE WHEN highway IN ('motorway', 'trunk') THEN 2 ELSE 1 END
                END + COALESCE(n_rev, 0) AS lanes_bwd,
                tags,
                -- A way that is one-way against its drawing (OSM -1) is turned round here, so every
                -- later step sees its legal direction as the forward one: edges, merging, restrictions
                -- and edge_id. ponytail: its direction-specific tags (turn:lanes:forward,
                -- cycleway:left/right) aren't swapped with it; rare on one-way roads.
                CASE WHEN dir = -1 THEN list_reverse(refs) ELSE refs END AS refs
            FROM base
        """)

    def _access_expression(self) -> str:
        """The value of the most specific access tag for the mode (OSM's hierarchy), or NULL."""
        if self.mode == "driving":
            car = _most_specific("motorcar", "motor_vehicle", "vehicle", "access")
            bus_ok = (f"({_tag('bus')} IN ('yes', 'designated') "
                      f"OR {_tag('psv')} IN ('yes', 'designated'))")
            # a road only buses (or taxis, psv) may use: 'bus', kept like a private road
            return f"""CASE WHEN {_tag('highway')} = 'busway' OR {car} = 'psv'
                                  OR ({car} IN ('no', 'private') AND {bus_ok}) THEN 'bus'
                             ELSE {car} END"""
        if self.mode == "walking":
            return _most_specific("foot", "access")
        ride = _most_specific("bicycle", "vehicle", "access")
        if not self.cycling_dismount:
            return ride
        # a dismount way (a footway / pedestrian street you may not ride) is walked: walking access
        return f"""CASE WHEN {_tag('highway')} IN ('footway', 'pedestrian')
                         AND COALESCE({_tag('bicycle')}, '') NOT IN ('yes', 'designated', 'permissive')
                        THEN {_most_specific('foot', 'access')} ELSE {ride} END"""

    def _direction_expression(self) -> str:
        """SQL integer expression for the travel direction, tailored to the mode: 1 = one-way in
        the way's drawing direction, -1 = one-way against it (OSM ``-1``), 0 = two-way.

        Roundabouts and OSM ``oneway`` in (yes/1/true) are one-way as drawn, ``-1`` against.
        ``motorway`` / ``motorway_link`` are **implicitly** one-way per the OSM convention even
        when untagged, unless an explicit ``oneway=no`` says otherwise. For cycling,
        ``oneway:bicycle`` overrides the generic ``oneway`` so contraflow cycling on one-way
        streets (very common in Europe) yields a reverse edge. For walking, vehicular ``oneway``
        is ignored entirely (pedestrians may walk either way); only an explicit ``oneway:foot``
        makes a walking edge one-way.
        """
        yes = "IN ('yes', '1', 'true')"
        if self.mode == "walking":
            return f"""
                CASE
                    WHEN map_extract(tags, 'oneway:foot')[1] {yes} THEN 1
                    WHEN map_extract(tags, 'oneway:foot')[1] = '-1' THEN -1
                    ELSE 0
                END
            """
        if self.mode == "cycling":
            # A dismount way is walked, not ridden — bidirectional regardless of any
            # oneway/oneway:bicycle tag (which governs riding). Rideable footways
            # (bicycle=yes/designated/permissive) keep the normal rules below.
            dismount_case = """
                    WHEN map_extract(tags, 'highway')[1] IN ('footway', 'pedestrian')
                         AND COALESCE(map_extract(tags, 'bicycle')[1], '')
                             NOT IN ('yes', 'designated', 'permissive') THEN 0
            """ if self.cycling_dismount else ""
            return f"""
                CASE
                    {dismount_case}
                    WHEN map_extract(tags, 'oneway:bicycle')[1] = 'no' THEN 0
                    WHEN map_extract(tags, 'oneway:bicycle')[1] {yes} THEN 1
                    WHEN map_extract(tags, 'oneway:bicycle')[1] = '-1' THEN -1
                    WHEN map_extract(tags, 'junction')[1] IN ('roundabout', 'circular') THEN 1
                    WHEN map_extract(tags, 'oneway')[1] {yes} THEN 1
                    WHEN map_extract(tags, 'oneway')[1] = '-1' THEN -1
                    ELSE 0
                END
            """
        return f"""
            CASE
                WHEN map_extract(tags, 'junction')[1] IN ('roundabout', 'circular') THEN 1
                WHEN map_extract(tags, 'oneway')[1] {yes} THEN 1
                WHEN map_extract(tags, 'oneway')[1] = '-1' THEN -1
                -- an explicit oneway=no wins over the implicit motorway rule below
                WHEN map_extract(tags, 'oneway')[1] IN ('no', 'false', '0') THEN 0
                -- motorways / motorway slip roads are implicitly one-way even when untagged
                WHEN map_extract(tags, 'highway')[1] IN ('motorway', 'motorway_link') THEN 1
                ELSE 0
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
