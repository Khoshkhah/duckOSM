"""The level editor of a duckOSM database (``duckosm levels-edit DB``): roadstyle's editor, started on the area of the file, with mapstyle's
base map (water, land, buildings, railways, signals) and every road of the file: the roads of all its modes and their ``private_edges``
(bus, private, pedestrian, delivery). A file with one mode gives a simple editor of that mode; with several, the editor's own boxes show only some.
Each Apply writes into the file (docs/design/levels_in_build.md)."""
from __future__ import annotations

import logging
from pathlib import Path

from duckosm.levels import is_current, load_roads, make_and_solve, roadstyle_module, sync_inputs

logger = logging.getLogger(__name__)


def items_for(db):
    """``f(roads)`` for roadstyle's editor (``serve(items=f)``): the overlays of mapstyle's feature layers (water, land, buildings, …)
    of ``db`` and the blank base map. ``None`` if mapstyle is not installed or the file has no ``features.*`` (the editor is then plain)."""
    try:
        from mapstyle.map import feature_overlays, load_layers
    except ImportError:
        logger.warning('the editor has no base map layers: pip install "duckosm[viz]" for mapstyle')
        return None

    def items(roads):
        overlays, _config = feature_overlays(load_layers(db))
        for o in overlays:
            o.visible = True                    # mapstyle's layers.js shows them once styled; in the editor they start visible
        return overlays, {"basemap": "blank"}
    return items


def prepare(db, area=None):
    """The area folder of ``db`` ready to edit: its files written from the tables (the master), and solved if the stored numbers are not those of
    the roads in the file. Returns the folder."""
    roadstyle_module()
    area = Path(area) if area else Path(db).with_suffix(".levels")
    roads = load_roads(db)
    if is_current(db, area, roads):
        sync_inputs(db, area, roads)
    else:
        res = make_and_solve(db, area)
        logger.info(f"level area {area}: {res['roads']:,} roads solved")
    return area


def edit(db, area=None, port=8780, plain=False):
    """Prepare the area of ``db`` and serve the editor at http://localhost:``port``/ until Ctrl+C. ``plain``: no mapstyle layers."""
    from roadstyle.level_editor import serve

    area = prepare(db, area)
    serve(area, port, items=None if plain else items_for(db))
