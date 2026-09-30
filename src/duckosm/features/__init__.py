"""OSM base-map feature extraction — the ``features.*`` schema (Shortbread layers).

duckOSM extracts every OSM theme duckmap needs (roads, water, land, buildings, POIs, transit,
places, …) once, into the same db as the routing graphs. duckmap consumes ``features.*`` for
rendering only. See docs/reference/features.md.
"""

from duckosm.features.builder import FeaturesBuilder
from duckosm.features.geometry import (build_area_layer, build_line_layer, build_point_layer,
                                       drop_foundation, ensure_foundation)
from duckosm.features.layers import LAYERS, Layer

__all__ = ["FeaturesBuilder", "LAYERS", "Layer", "ensure_foundation", "drop_foundation",
           "build_area_layer", "build_line_layer", "build_point_layer"]
