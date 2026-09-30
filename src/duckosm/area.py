"""Prepare an area before a build: find its boundary by name, and cut a PBF down to it.

    duckosm boundary "Monaco" --pbf monaco-latest.osm.pbf      # -> monaco.geojson
    duckosm clip-pbf monaco-latest.osm.pbf monaco.geojson       # -> monaco.osm.pbf

A boundary is looked up in the PBF's own borders first (OSM boundary relations, offline, via GDAL's
ogr2ogr), then with OpenStreetMap's Nominatim search (online).
"""
import json
import logging
import shutil
import subprocess
import tempfile
import urllib.parse
import urllib.request
from pathlib import Path

import duckdb

logger = logging.getLogger("duckosm")

NOMINATIM = "https://nominatim.openstreetmap.org"


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
    """Return ``(geometry_geojson, info)`` for the area called ``name`` (or OSM relation ``osm_id``).

    Looks in ``pbf``'s own borders first, then asks Nominatim unless ``offline``. ``info`` holds
    ``name``, ``osm_id``, ``admin_level``, ``area_km2``, ``source`` and ``others``: the other borders
    that matched the name (best first), so an ambiguous name can be resolved with ``osm_id``.
    Raises LookupError."""
    if pbf:
        if shutil.which("ogr2ogr"):
            hit = _from_pbf(Path(pbf), name, osm_id)
            if hit:
                return hit
            logger.info(f"  no border matching {name or osm_id!r} in {Path(pbf).name}")
        else:
            logger.warning("  ogr2ogr (GDAL) not found: can't read the PBF's borders")
    if not offline:
        hit = _from_nominatim(name, osm_id)
        if hit:
            return hit
    where = f"in {Path(pbf).name}" + ("" if offline else " or on Nominatim") if pbf else "on Nominatim"
    raise LookupError(f"no boundary found for {name or f'OSM relation {osm_id}'!r} {where}. "
                      "Try a more specific name (e.g. 'Södermalm, Stockholm') or --osm-id.")


def _from_pbf(pbf, name, osm_id):
    from duckosm.admin import extract_boundaries, load_boundaries

    with tempfile.TemporaryDirectory() as tmp:
        gpkg, db = Path(tmp) / "admin.gpkg", Path(tmp) / "admin.duckdb"
        extract_boundaries(pbf, gpkg)
        load_boundaries(gpkg, db)
        con = duckdb.connect(str(db))
        con.execute("LOAD spatial")
        pick = "SELECT ST_AsGeoJSON(geom), osm_id, name, admin_level, " \
               "ST_Area_Spheroid(ST_FlipCoordinates(geom)) / 1e6 FROM "
        if osm_id is not None:
            rows = con.execute(pick + "(SELECT geometry AS geom, osm_id, name, admin_level "
                               f"FROM admin_boundaries WHERE osm_id = {int(osm_id)})").fetchall()
        else:
            rows = con.execute(pick + f"({admin_match_sql('admin_boundaries', limit=6)})",
                               {"q": name}).fetchall()
        con.close()
    if not rows:
        return None
    info = lambda r: {"name": r[2], "osm_id": r[1], "admin_level": r[3], "area_km2": round(r[4], 2)}
    return rows[0][0], {**info(rows[0]), "source": f"PBF {pbf.name}",
                        "others": [info(r) for r in rows[1:]]}


def _from_nominatim(name, osm_id):
    from duckosm import __version__

    if osm_id is not None:
        url = f"{NOMINATIM}/lookup?" + urllib.parse.urlencode(
            {"osm_ids": f"R{int(osm_id)}", "format": "geojson", "polygon_geojson": 1})
    else:
        url = f"{NOMINATIM}/search?" + urllib.parse.urlencode(
            {"q": name, "format": "geojson", "polygon_geojson": 1, "limit": 5})
    req = urllib.request.Request(url, headers={
        "User-Agent": f"duckOSM/{__version__} (+https://github.com/Khoshkhah/duckOSM)"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            features = json.load(r).get("features", [])
    except OSError as e:
        logger.warning(f"  Nominatim not reachable: {e}")
        return None
    found = []                                            # a place can come back as a point: skip those
    for f in features:
        if f.get("geometry", {}).get("type") in ("Polygon", "MultiPolygon"):
            p, geom = f["properties"], json.dumps(f["geometry"])
            area = duckdb.execute("INSTALL spatial; LOAD spatial; SELECT ST_Area_Spheroid("
                                  "ST_FlipCoordinates(ST_GeomFromGeoJSON(?))) / 1e6", [geom]).fetchone()[0]
            found.append((geom, {"name": p.get("display_name") or p.get("name"), "osm_id": p.get("osm_id"),
                                 "admin_level": None, "area_km2": round(area, 2)}))
    if not found:
        return None
    return found[0][0], {**found[0][1], "source": "Nominatim", "others": [i for _, i in found[1:]]}


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
    """Cut ``pbf`` down to ``boundary`` (GeoJSON) with osmium, writing ``out``.

    ``complete_ways`` (default) keeps every way that touches the area whole, with all its nodes, so
    no junction at the border is lost; ``smart`` also completes multipolygon relations (rivers,
    land cover), which a base-map (features) build needs. Raises RuntimeError on failure."""
    osmium = shutil.which("osmium")
    if not osmium:
        raise RuntimeError("osmium not found (install osmium-tool)")
    Path(out).parent.mkdir(parents=True, exist_ok=True)
    r = subprocess.run([osmium, "extract", "--strategy", strategy, "--polygon", str(boundary),
                        "--output", str(out), "--overwrite", str(pbf)], capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"osmium extract failed: {r.stderr.strip()[:300]}")
    return Path(out)
