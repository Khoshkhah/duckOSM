"""GlobalJunctions — a mode-agnostic road-junction set for cross-mode ``edge_id`` alignment.

Builds ``main.global_junctions(node_id, is_road_junction)`` from the **full raw network** — every
highway way in ``raw.ways`` (roads and footways / paths / cycleways alike), *before* any per-mode
access / oneway / class filtering. A road is cut where another OSM way uses its node, including a
footway or crosswalk joining it mid-way, so walking and cycling connect there; and it is cut the same
way in every mode (docs/design/split_at_every_way.md).
``GraphSimplifier`` then segments every mode's roads at this one shared junction set, so a physical
stretch of road is split the same way — and carries the **same ``edge_id``** — in the
driving / walking / cycling graphs.

Without it, a road-class way that one mode drops (e.g. a ``service`` driveway removed from cycling by
``access=private``) makes a shared node a junction in one mode but not another, so the crossing road
is segmented differently per mode and its ``edge_id`` diverges. See
``docs/design/global_junction_segmentation.md`` (Kalevi example).

Runs **once** (a global pre-pass, before the per-mode loop), writing to the ``main`` schema so every
mode reads the same table.
"""
import logging

from duckosm.processors.base import BaseProcessor

logger = logging.getLogger(__name__)

# Non-road (non-vehicular) highway classes — MUST match GraphSimplifier._NON_ROAD_HIGHWAYS. They cut a
# road where they share its node, but their own ends don't (a path ending in the open is not a junction).
_NON_ROAD_HIGHWAYS = "('footway','path','cycleway','steps','pedestrian','bridleway','corridor')"
# Highway ways that don't exist on the ground: they never cut a road.
_NOT_BUILT = "('proposed','construction','abandoned','razed','disused')"


class GlobalJunctions(BaseProcessor):
    """Build ``main.global_junctions`` (the nodes that are road junctions across ALL modes)."""

    def run(self) -> None:
        # A node is a global road junction when >=2 highway way references meet there (a road, or a
        # footway / path / cycleway), or a road-class way ends there. Counting references (not
        # distinct ways) matches the per-mode _find_junctions semantics and also flags self-crossing
        # nodes (a way revisiting a node).
        self.execute(f"""
            CREATE OR REPLACE TABLE main.global_junctions AS
            WITH hw_ways AS (
                SELECT refs, map_extract(tags, 'highway')[1] NOT IN {_NON_ROAD_HIGHWAYS} AS is_road
                FROM raw.ways
                WHERE map_extract(tags, 'highway')[1] IS NOT NULL
                  AND map_extract(tags, 'highway')[1] NOT IN {_NOT_BUILT}
            ),
            node_use AS (
                SELECT unnest(refs) AS node_id,
                       generate_subscripts(refs, 1) AS pos,
                       len(refs) AS n,
                       is_road
                FROM hw_ways
            ),
            agg AS (
                SELECT node_id,
                       count(*) AS way_count,
                       sum(CASE WHEN is_road THEN 1 ELSE 0 END) AS road_way_count,
                       sum(CASE WHEN is_road AND (pos = 1 OR pos = n) THEN 1 ELSE 0 END) AS road_endpoint_count
                FROM node_use
                GROUP BY node_id
            )
            SELECT node_id, TRUE AS is_road_junction
            FROM agg
            WHERE road_way_count > 0 AND (way_count > 1 OR road_endpoint_count > 0)
        """)
        n = self.fetchone("SELECT COUNT(*) FROM main.global_junctions")[0]
        logger.info(f"  Global junctions: {n:,} mode-agnostic road junctions")
