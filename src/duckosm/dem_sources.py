"""The DEM sources that duckOSM knows, and the ones that cover an area.

The list is `data/dem_sources.json`: one entry for each source, with its coverage, resolution, product (DTM or
DSM), CRS, vertical datum, licence and access. The page "Ground" of duckOSM Studio (`web/ground_step/`) reads
the same file. `docs/reference/elevation-sources.md` has two tables that `render_tables()` makes from it
(`python scripts/update_elevation_sources_doc.py`; a test checks them).

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


def _in_ring(lon: float, lat: float, ring) -> bool:
    inside = False
    for (x1, y1), (x2, y2) in zip(ring, ring[1:] + ring[:1]):
        if (y1 > lat) != (y2 > lat) and lon < (x2 - x1) * (lat - y1) / (y2 - y1) + x1:
            inside = not inside
    return inside


def covers(source: dict, bbox) -> bool:
    """True when the whole box `(west, south, east, north)` is inside the coverage of the source and touches none
    of its `exclude` boxes. A polygon is coarse: the four corners must be inside it."""
    w, s, e, n = bbox
    cov = source["coverage"]
    if "bbox" in cov:
        cw, cs, ce, cn = cov["bbox"]
        ok = cw <= w and e <= ce and cs <= s and n <= cn
    else:
        ok = all(_in_ring(x, y, cov["polygon"]) for x in (w, e) for y in (s, n))
    return ok and not any(w < xe and xw < e and s < yn and ys < n for xw, ys, xe, yn in cov.get("exclude", []))


def sources_for_bbox(bbox, *, include_fallback: bool = False, include_all: bool = False) -> list[dict]:
    """The sources that cover the box, best first: a national 1 m DTM, then EU-DTM, FABDEM, Copernicus GLO-30, then the
    others. The order is `rank`, then the resolution, then the order of the file. A source with `suggest: false` (too
    coarse for roads) is left out unless `include_all`; the fallback (AWS Terrain Tiles) is left out unless
    `include_fallback`."""
    keep = [s for s in load()
            if covers(s, bbox)
            and (include_all or s.get("suggest", True))
            and (include_fallback or not s.get("fallback"))]
    return sorted(keep, key=lambda s: (s["rank"], s["resolution_m"]))      # sorted() keeps the file order on a tie


def get(source_id: str) -> dict:
    """The entry with this `id`."""
    for s in load():
        if s["id"] == source_id:
            return s
    raise KeyError(source_id)


def _row(cells) -> str:
    return "| " + " | ".join(cells) + " |"


def render_tables() -> dict[str, str]:
    """The two tables of `docs/reference/elevation-sources.md` that come from the catalog: `global` (the models that
    `--dem` reads and `--source` does not fetch) and `national`."""
    g = [_row(["Model", "Resolution", "Covers", "Type", "Access", "Licence"]), _row(["---"] * 6)]
    n = [_row(["Country", "Model", "Resolution", "CRS (coordinate system)", "Access", "Licence"]), _row(["---"] * 6)]
    for s in load():
        a = s["access"]
        if s["rank"] == 1:
            n.append(_row([s["country"], s["name"], s["resolution_text"] + " " + s["product_text"],
                           f"{s['crs']}, heights {s['vertical_datum']}", a["text"], s["license_text"]]))
        elif s["rank"] < 9 and not s["duckosm_source"]:
            g.append(_row([s["name"], s["resolution_text"], s["covers_text"], s["product_text"], a["text"], s["license_text"]]))
    return {"global": "\n".join(g), "national": "\n".join(n)}
