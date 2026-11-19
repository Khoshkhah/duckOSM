"""Prepare an area before a build: find its boundary by name, and cut a PBF down to it.

    duckosm boundary "Monaco" --pbf monaco-latest.osm.pbf      # -> monaco.geojson
    duckosm clip-pbf monaco-latest.osm.pbf monaco.geojson       # -> monaco.osm.pbf

Both call the Rust program ``duckosm``. A boundary is looked up in the PBF's own borders first (OSM boundary
relations, offline), then with OpenStreetMap's Nominatim search (online).
"""
import json
import tempfile
from pathlib import Path


def _cli(*args):
    """Run the Rust program ``duckosm`` with ``args``; give its exit code."""
    from duckosm import _rust
    return _rust.cli(["duckosm", *map(str, args)])


def admin_match_sql(table, has_en=True, limit=1):
    """SELECT the best admin boundary in ``table`` for the name ``$q``: case, accents and hyphens
    ignored ("monte carlo" finds Monte-Carlo), ``name:en`` matched too, an exact name before a
    partial one, then the largest area. Shared by ``extract --name`` and ``duckosm boundary``."""
    norm = lambda x: f"replace(strip_accents(lower({x})), '-', ' ')"
    q, nm = norm("$q"), norm("name")
    en = norm("coalesce(name_en, '')") if has_en else "''"
    return (f"SELECT geometry AS geom, osm_id, name, admin_level FROM {table} "
            f"WHERE {nm} LIKE '%' || {q} || '%' OR {en} LIKE '%' || {q} || '%' "
            f"ORDER BY ({nm} = {q} OR {en} = {q}) DESC, ST_Area(geometry) DESC LIMIT {int(limit)}")


def find_boundary(name=None, pbf=None, osm_id=None, offline=False):
    """Return ``(geometry_geojson, info)`` for the area called ``name`` (or OSM relation ``osm_id``), from the Rust
    ``duckosm boundary``.

    Looks in ``pbf``'s own borders first, then asks Nominatim unless ``offline``. ``info`` holds ``name``, ``osm_id``,
    ``admin_level``, ``area_km2`` and ``source``. Raises LookupError."""
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / "boundary.geojson"
        args = [name] if name else []
        args += ["--pbf", pbf] if pbf else []
        args += ["--osm-id", int(osm_id)] if osm_id is not None else []
        args += ["--offline"] if offline else []
        if _cli("boundary", *args, "-o", out) != 0 or not out.exists():
            raise LookupError(f"no boundary found for {name or f'OSM relation {osm_id}'!r}")
        feature = json.loads(out.read_text())["features"][0]
    return json.dumps(feature["geometry"]), {**feature["properties"], "others": []}


def bbox_geojson(bbox):
    """A ``[west, south, east, north]`` box (degrees) as a GeoJSON Polygon string."""
    try:
        w, s, e, n = (float(v) for v in bbox)
    except (TypeError, ValueError):
        raise ValueError(f"boundary.bbox must be [west, south, east, north], got {bbox!r}")
    if not (-180 <= w < e <= 180 and -90 <= s < n <= 90):
        raise ValueError(f"boundary.bbox must be [west, south, east, north] in degrees, got {bbox!r}")
    return json.dumps({"type": "Polygon", "coordinates": [[[w, s], [e, s], [e, n], [w, n], [w, s]]]})


def h3_geojson(cell):
    """The outline of H3 cell ``cell`` (hex string) as a GeoJSON Polygon string."""
    import h3
    if not h3.is_valid_cell(str(cell)):
        raise ValueError(f"not an H3 cell: {cell!r}")
    ring = [[lng, lat] for lat, lng in h3.cell_to_boundary(str(cell))]
    return json.dumps({"type": "Polygon", "coordinates": [ring + ring[:1]]})


def write_boundary(geometry_geojson, info, out):
    """Write a boundary as a one-feature GeoJSON FeatureCollection (``info`` as its properties)."""
    out = Path(out)
    out.parent.mkdir(parents=True, exist_ok=True)
    props = {k: v for k, v in info.items() if k != "others"}
    out.write_text(json.dumps({"type": "FeatureCollection", "features": [
        {"type": "Feature", "properties": props, "geometry": json.loads(geometry_geojson)}]}))
    return out


def clip_pbf(pbf, boundary, out, strategy="complete_ways"):
    """Cut ``pbf`` down to ``boundary`` (GeoJSON) with the Rust ``duckosm clip-pbf``, writing ``out``.

    ``complete_ways`` (default) keeps every way that touches the area whole, with all its nodes, so
    no junction at the border is lost; ``smart`` also completes multipolygon relations (rivers,
    land cover), which a base-map (features) build needs. Raises RuntimeError on failure."""
    if _cli("clip-pbf", pbf, boundary, "-o", out, "--strategy", strategy) != 0:
        raise RuntimeError(f"duckosm clip-pbf failed on {pbf}")
    return Path(out)
