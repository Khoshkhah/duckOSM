"""FeaturesBuilder — build the ``features.*`` schema (Shortbread base-map layers) from ``raw.*``.

The map-rendering half of duckOSM: everything duckmap needs, extracted **once** into the same db
as the routing graphs. duckmap then reads ``features.*`` and only styles/renders — it no longer
ingests OSM or builds its own db.

Flow: ``raw`` -> geometry foundation (``geom.*``) -> thematic ``features.*`` layers -> drop ``geom``.
Dropping the scaffolding means each geometry ends up in exactly one place (its feature table); no
lasting duplication (see docs/features_schema.md).
"""

import logging

from duckosm.features.geometry import drop_foundation, ensure_foundation
from duckosm.features.layers import LAYERS

logger = logging.getLogger("duckosm")


class FeaturesBuilder:
    """Build ``features.*`` on a db that already has ``raw.*``.

    Parameters
    ----------
    con : writable DuckDB connection to a built (or raw-loaded) duckOSM db.
    layers : optional set of layer names to build (default: every ``enabled`` layer).
    keep_geom : keep the ``geom.*`` scaffolding after building (default drop it).
    """

    def __init__(self, con, layers=None, keep_geom: bool = False):
        self.con = con
        self.wanted = set(layers) if layers else None
        self.keep_geom = keep_geom
        self.stats = {}

    def run(self) -> dict:
        self.con.execute("INSTALL spatial; LOAD spatial;")
        logger.info("[features] building geometry foundation ...")
        ensure_foundation(self.con)

        logger.info("[features] building layers ...")
        for layer_cls in LAYERS:
            layer = layer_cls(self.con)
            if self.wanted is None:
                if not layer.enabled:
                    continue
            elif layer.name not in self.wanted:
                continue
            try:
                self.stats[layer.name] = layer.build()
            except Exception as e:  # one bad layer shouldn't sink the whole features build
                logger.warning("[features] layer %s FAILED: %s", layer.name,
                               str(e).splitlines()[0])
                self.stats[layer.name] = -1

        if not self.keep_geom:
            drop_foundation(self.con)

        built = [k for k, v in self.stats.items() if v and v >= 0]
        logger.info("[features] built: %s", ", ".join(built) or "(none)")
        return self.stats
