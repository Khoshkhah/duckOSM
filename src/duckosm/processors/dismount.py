"""DismountMarker — flag the cycling edges that are walked, not ridden.

Adds ``dismount BOOLEAN`` to the cycling ``edges``: TRUE for footway/pedestrian edges whose
raw way does not explicitly allow riding (``bicycle`` not in yes/designated/permissive).
These are the push-the-bike connectors that keep the cycling graph connected (and its
cycleway fragments alive through the component clean-up) while staying legally correct —
``SpeedProcessor`` costs them at walking speed. Runs after the graph is built and before
speeds (``raw.ways`` must still be present); synthetic connector edges (``osm_id < 0``, no
raw way) stay FALSE. See ``docs/design/cycling_dismount_edges.md``.
"""
import logging

from duckosm.processors.base import BaseProcessor

logger = logging.getLogger(__name__)


class DismountMarker(BaseProcessor):
    """Add ``dismount`` to the active (cycling) schema's ``edges``."""

    def run(self) -> None:
        self.execute("ALTER TABLE edges ADD COLUMN IF NOT EXISTS dismount BOOLEAN DEFAULT FALSE")
        self.execute("""
            UPDATE edges SET dismount = TRUE
            FROM raw.ways w
            WHERE w.osm_id = edges.osm_id
              AND edges.highway IN ('footway', 'pedestrian')
              AND COALESCE(map_extract(w.tags, 'bicycle')[1], '')
                  NOT IN ('yes', 'designated', 'permissive')
        """)
        n, total = self.fetchall(
            "SELECT count(*) FILTER (WHERE dismount), count(*) FROM edges")[0]
        logger.info(f"  dismount: {n:,}/{total:,} edges (walked, not ridden)")
