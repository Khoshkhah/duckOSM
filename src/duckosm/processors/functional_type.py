"""FunctionalType — derive a pedestrian/cyclist functional class onto the mode's edges.

``highway`` alone is too coarse for walking/cycling: the mode-critical distinction lives in an OSM
*sub-tag* (``footway=sidewalk|crossing``, ``cycleway:<side>=lane|track|…``) that ``highway`` doesn't
carry. This processor adds one derived enum column by joining each edge back to its raw OSM way tags
on ``osm_id``:

* walking → ``walk_type``  (sidewalk / crossing / footpath / steps / escalator / pedestrian_street /
  plaza / shared_street / shared_road / corridor / platform / path)
* cycling → ``cycle_type`` (cycleway / cycle_track / cycle_lane / shared_lane / bus_cycle_lane /
  segregated_path / shared_path / mixed_traffic) — **directional**: reads ``cycleway:right`` on
  forward edges and ``cycleway:left`` on reverse edges (``is_reverse``), like ``lanes``.

Runs after the graph is simplified (``raw.ways`` must still be present). Synthetic connector edges
(``osm_id < 0``, no raw way) fall back to a highway-only class. See ``docs/design/walk_cycle_type.md``.
"""
import logging

from duckosm.processors.base import BaseProcessor

logger = logging.getLogger(__name__)


class FunctionalType(BaseProcessor):
    """Add ``walk_type`` (walking) or ``cycle_type`` (cycling) to the active schema's ``edges``."""

    def __init__(self, con, mode: str):
        super().__init__(con)
        self.mode = mode

    def run(self) -> None:
        if self.mode == "walking":
            self._walk_type()
        elif self.mode == "cycling":
            self._cycle_type()

    def _walk_type(self) -> None:
        self.execute("ALTER TABLE edges ADD COLUMN IF NOT EXISTS walk_type VARCHAR")
        # From highway + footway/conveying/area/indoor on the raw way.
        self.execute("""
            UPDATE edges SET walk_type = CASE
                WHEN edges.highway = 'steps' AND map_extract(w.tags, 'conveying')[1] IS NOT NULL THEN 'escalator'
                WHEN edges.highway = 'steps'                                          THEN 'steps'
                WHEN edges.highway = 'footway' AND map_extract(w.tags, 'footway')[1] = 'sidewalk' THEN 'sidewalk'
                WHEN edges.highway = 'footway' AND map_extract(w.tags, 'footway')[1] = 'crossing' THEN 'crossing'
                WHEN edges.highway = 'footway'                                        THEN 'footpath'
                WHEN edges.highway = 'pedestrian' AND map_extract(w.tags, 'area')[1] = 'yes' THEN 'plaza'
                WHEN edges.highway = 'pedestrian'                                     THEN 'pedestrian_street'
                WHEN edges.highway = 'living_street'                                  THEN 'shared_street'
                WHEN edges.highway IN ('residential', 'service', 'unclassified')      THEN 'shared_road'
                WHEN edges.highway = 'corridor' OR map_extract(w.tags, 'indoor')[1] = 'yes' THEN 'corridor'
                WHEN edges.highway = 'platform'                                       THEN 'platform'
                WHEN edges.highway IN ('path', 'track', 'bridleway')                  THEN 'path'
                ELSE 'footpath'
            END
            FROM raw.ways w WHERE w.osm_id = edges.osm_id
        """)
        # Connector / tagless edges (no matching raw way): highway-only fallback.
        self.execute("""
            UPDATE edges SET walk_type = CASE
                WHEN highway = 'steps'                                    THEN 'steps'
                WHEN highway = 'pedestrian'                               THEN 'pedestrian_street'
                WHEN highway = 'living_street'                            THEN 'shared_street'
                WHEN highway IN ('residential', 'service', 'unclassified') THEN 'shared_road'
                WHEN highway = 'corridor'                                 THEN 'corridor'
                WHEN highway = 'platform'                                 THEN 'platform'
                WHEN highway IN ('path', 'track', 'bridleway')            THEN 'path'
                ELSE 'footpath'
            END
            WHERE walk_type IS NULL
        """)
        self._log("walk_type")

    def _cycle_type(self) -> None:
        self.execute("ALTER TABLE edges ADD COLUMN IF NOT EXISTS cycle_type VARCHAR")
        # cw_side = the cycleway tag on the side matching the edge's direction of travel:
        # forward reads cycleway:right, reverse reads cycleway:left; both fall back to
        # cycleway:both / cycleway.
        cw_side = """COALESCE(
            CASE WHEN edges.is_reverse THEN map_extract(w.tags, 'cycleway:left')[1]
                 ELSE map_extract(w.tags, 'cycleway:right')[1] END,
            map_extract(w.tags, 'cycleway:both')[1],
            map_extract(w.tags, 'cycleway')[1])"""
        self.execute(f"""
            UPDATE edges SET cycle_type = CASE
                WHEN edges.highway = 'cycleway'                          THEN 'cycleway'
                WHEN {cw_side} = 'track'                                 THEN 'cycle_track'
                WHEN {cw_side} = 'lane'                                  THEN 'cycle_lane'
                WHEN {cw_side} = 'shared_lane'                           THEN 'shared_lane'
                WHEN {cw_side} = 'share_busway'                          THEN 'bus_cycle_lane'
                WHEN edges.highway IN ('path', 'track', 'bridleway')
                     AND map_extract(w.tags, 'segregated')[1] = 'yes'    THEN 'segregated_path'
                WHEN edges.highway IN ('path', 'track', 'bridleway')     THEN 'shared_path'
                ELSE 'mixed_traffic'
            END
            FROM raw.ways w WHERE w.osm_id = edges.osm_id
        """)
        # Connector / tagless edges (no matching raw way): highway-only fallback.
        self.execute("""
            UPDATE edges SET cycle_type = CASE
                WHEN highway = 'cycleway'                        THEN 'cycleway'
                WHEN highway IN ('path', 'track', 'bridleway')   THEN 'shared_path'
                ELSE 'mixed_traffic'
            END
            WHERE cycle_type IS NULL
        """)
        self._log("cycle_type")

    def _log(self, col: str) -> None:
        rows = self.fetchall(f"SELECT {col}, count(*) c FROM edges GROUP BY 1 ORDER BY c DESC")
        summary = ", ".join(f"{v}:{c}" for v, c in rows[:6])
        logger.info(f"  {col}: {summary}")
