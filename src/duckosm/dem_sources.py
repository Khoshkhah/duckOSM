"""The DEM sources that duckOSM knows, and the ones that cover an area.

The list is `data/dem_sources.json`: one entry for each source, with its coverage, resolution, product (DTM or
DSM), CRS, vertical datum, licence and access. The page "Ground" of duckOSM Studio (`web/ground_step/`) reads
the same file. `docs/reference/elevation-sources.md` has two tables that `render_tables()` makes from it
(`python scripts/update_elevation_sources_doc.py` makes them; a test checks them).

    >>> [s["id"] for s in sources_for_bbox((7.41, 43.73, 7.44, 43.75))][:3]
    ['eudtm', 'fabdem', 'copernicus-glo30']
"""
from __future__ import annotations

import json
from functools import lru_cache
from importlib import resources


@lru_cache(maxsize=1)
def load() -> list[dict]:
    """All the entries of `dem_sources.json`, in the order of the file."""
    text = resources.files("duckosm").joinpath("data/dem_sources.json").read_text(encoding="utf-8")
    return json.loads(text)["sources"]


def covers(source: dict, bbox) -> bool:
    """True when the whole box `(west, south, east, north)` is inside the coverage of the source and touches none
    of its `exclude` boxes. A polygon is coarse: the four corners must be inside it (Rust)."""
    from duckosm import _rust
    return _rust.dem_source_covers(source["id"], [float(v) for v in bbox])


def sources_for_bbox(bbox, *, include_fallback: bool = False, include_all: bool = False) -> list[dict]:
    """The sources that cover the box, best first: a national 1 m DTM, then EU-DTM, FABDEM, Copernicus GLO-30, then the
    others. The order is `rank`, then the resolution, then the order of the file. A source with `suggest: false` (too
    coarse for roads) is left out unless `include_all`; the fallback (AWS Terrain Tiles) is left out unless
    `include_fallback` (Rust)."""
    from duckosm import _rust
    return [get(i) for i in _rust.dem_sources_for_bbox([float(v) for v in bbox], include_fallback, include_all)]


def get(source_id: str) -> dict:
    """The entry with this `id`."""
    for s in load():
        if s["id"] == source_id:
            return s
    raise KeyError(source_id)
