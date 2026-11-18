"""
GIS-export debug visualization — read an *exported* GeoPackage / shapefile back **through GDAL**
(exactly as a GIS consumer like QGIS / ArcGIS / FME would) and emit a single self-contained HTML
page that renders every layer and audits it. This verifies the export *file itself*, independent of
duckOSM / DuckDB — so it is deliberately NOT the roadstyle path (roadstyle inspects the network in
the db; this inspects what actually landed on disk).

The page has two halves:

  * a **canvas map** of the exported geometry — edges coloured by OSM highway class, a nodes layer,
    the boundary outline, per-mode toggles, pan/zoom — drawn with no external tiles or libraries
    (CSP-safe, fully inlined), and
  * a **QA panel** — per-layer feature counts, geometry type, CRS, and the thing this export is all
    about: ``edge_id`` integrity (dtype + exactness — a float dtype means the shapefile DBF silently
    rounded the 64-bit hash). If a ``source_db`` is given it also runs a **round-trip diff**: the
    exported ``edge_id`` set vs the db's, per mode — the definitive "did every edge survive intact"
    check.

    from duckosm.gis_debug import write_debug
    write_debug("sodermalm.gpkg", source_db="data/db/sodermalm.duckdb", out="reports/gis_debug.html")

Needs ``geopandas`` (+ ``pyogrio``) to read the export. ``duckdb`` only when ``source_db`` is set.
"""
import json
import logging
import math
from pathlib import Path

logger = logging.getLogger("duckosm")

# OSM highway class → (label, colour on the dark map, stroke width px). List order is z-order:
# minor roads first (drawn under), arterials last (drawn on top). Colours follow cartographic
# convention (motorway warm/bold → footway green → cycleway blue), tuned to read on deep slate.
_CLASSES = [
    ("path",          "Foot / path",   "#6f9e79", 1.0),
    ("cycleway",      "Cycleway",      "#5aa0d8", 1.2),
    ("service",       "Service",       "#5f7180", 1.0),
    ("track",         "Track",         "#a98a5f", 1.0),
    ("living_street", "Living street", "#8fa3b0", 1.4),
    ("residential",   "Residential",   "#9fb0bd", 1.4),
    ("unclassified",  "Unclassified",  "#9aa7b2", 1.4),
    ("tertiary",      "Tertiary",      "#d9dee2", 1.8),
    ("secondary",     "Secondary",     "#f4e08a", 2.1),
    ("primary",       "Primary",       "#f2c14e", 2.5),
    ("trunk",         "Trunk",         "#e8934a", 2.9),
    ("motorway",      "Motorway",      "#e8663c", 3.3),
    ("other",         "Other",         "#7a8894", 1.2),
]
_CLASS_IDX = {k: i for i, (k, *_ ) in enumerate(_CLASSES)}
_HW_TO_CLASS = {
    "motorway": "motorway", "trunk": "trunk", "primary": "primary", "secondary": "secondary",
    "tertiary": "tertiary", "residential": "residential", "unclassified": "unclassified",
    "living_street": "living_street", "service": "service", "track": "track",
    "cycleway": "cycleway", "footway": "path", "path": "path", "pedestrian": "path",
    "steps": "path", "bridleway": "path", "road": "unclassified",
}
_Q = 4096          # quantization grid (kept small: ~1 m resolution at city scale, tiny payload)


def _class_index(highway):
    if not highway:
        return _CLASS_IDX["other"]
    h = str(highway).split(";")[0].strip()
    if h.endswith("_link"):
        h = h[:-5]
    return _CLASS_IDX.get(_HW_TO_CLASS.get(h, "other"), _CLASS_IDX["other"])


def _parse_layer(layer_name):
    """(kind, mode) from a layer/shapefile-stem name: edges_<mode> / nodes_<mode> / …_boundary."""
    for kind in ("edges", "nodes"):
        tok = f"{kind}_"
        if tok in layer_name:
            return kind, layer_name.split(tok, 1)[1]
    if layer_name == "boundary" or layer_name.endswith("_boundary"):
        return "boundary", None
    return "other", None


def _discover(path):
    """(fmt, [(layer_name, source_path, gpkg_layer_or_None)]) for a .gpkg file or a shapefile dir."""
    p = Path(path)
    if p.is_dir():
        return "Shapefile", [(shp.stem, str(shp), None) for shp in sorted(p.glob("*.shp"))]
    fmt = "GeoPackage" if p.suffix.lower() == ".gpkg" else p.suffix.lstrip(".").upper()
    import pyogrio
    return fmt, [(str(r[0]), str(p), str(r[0])) for r in pyogrio.list_layers(str(p))]


def _read(gpd, source, gpkg_layer):
    return gpd.read_file(source, layer=gpkg_layer) if gpkg_layer else gpd.read_file(source)


def _iter_lines(geom):
    """Yield each LineString's coord list from a (Multi)LineString geometry."""
    if geom is None or geom.is_empty:
        return
    if geom.geom_type == "LineString":
        yield list(geom.coords)
    elif geom.geom_type == "MultiLineString":
        for part in geom.geoms:
            yield list(part.coords)


def _iter_rings(geom):
    """Yield exterior rings from a (Multi)Polygon boundary geometry."""
    if geom is None or geom.is_empty:
        return
    if geom.geom_type == "Polygon":
        yield list(geom.exterior.coords)
    elif geom.geom_type == "MultiPolygon":
        for part in geom.geoms:
            yield list(part.exterior.coords)


def build_payload(path, source_db=None, name=None):
    """Read the export back through GDAL and build the JSON payload the debug page renders.

    Returns a dict: file meta, projection, an overall verdict, per-layer QA rows, the legend, and the
    quantized geometry grouped by mode (+ boundary). Raises ImportError if geopandas is missing.
    """
    try:
        import geopandas as gpd
    except ImportError as e:
        raise ImportError("gis-debug needs geopandas + pyogrio (pip install geopandas pyogrio)") from e

    fmt, entries = _discover(path)
    if not entries:
        raise ValueError(f"no layers found in {path}")

    # first pass: read every layer, collect QA facts + the coord extent (for a shared projection)
    read_layers = []            # (layer_name, kind, mode, gdf)
    minx = miny = math.inf
    maxx = maxy = -math.inf
    for layer_name, source, gpkg_layer in entries:
        kind, mode = _parse_layer(layer_name)
        gdf = _read(gpd, source, gpkg_layer)
        read_layers.append((layer_name, kind, mode, gdf))
        if len(gdf) and gdf.geometry.notna().any():
            bx = gdf.total_bounds                      # [minx, miny, maxx, maxy]
            if not any(map(math.isinf, bx)) and not any(map(math.isnan, bx)):
                minx, miny = min(minx, bx[0]), min(miny, bx[1])
                maxx, maxy = max(maxx, bx[2]), max(maxy, bx[3])

    if math.isinf(minx):
        raise ValueError("export has no drawable geometry")

    # shared equirectangular projection (cos-lat corrected) → a square-ish quantization grid
    midlat = (miny + maxy) / 2.0
    kx = math.cos(math.radians(midlat)) or 1.0
    projW, projH = (maxx - minx) * kx, (maxy - miny)
    scale = _Q / max(projW, projH, 1e-9)
    gridW, gridH = round(projW * scale), round(projH * scale)

    def qx(x):
        return round((x - minx) * kx * scale)

    def qy(y):
        return round((maxy - y) * scale)                # flip: north is up

    # optional round-trip: exported edge_id set vs the source db's, per mode
    src_ids = {}
    if source_db:
        import duckdb
        con = duckdb.connect(str(source_db), read_only=True)
        con.execute("INSTALL spatial; LOAD spatial; SET geometry_always_xy = false;")
        modes = [r[0] for r in con.execute(
            "SELECT DISTINCT schema_name FROM duckdb_tables() WHERE table_name='edges' "
            "AND schema_name NOT IN ('information_schema','pg_catalog','main','raw')").fetchall()]
        for m in modes:
            src_ids[m] = {str(r[0]) for r in con.execute(
                f'SELECT CAST(edge_id AS VARCHAR) FROM "{m}".edges').fetchall()}
        con.close()

    modes_geo = {}              # mode -> {"edges":[[cidx,x0,y0,...]], "nodes":[x0,y0,...]}
    boundary_rings = []
    qa_rows = []
    total_features = 0
    crs_set = set()
    worst = "pass"              # verdict escalates pass -> warn -> fail

    def bump(level):
        nonlocal worst
        order = {"pass": 0, "warn": 1, "fail": 2}
        if order[level] > order[worst]:
            worst = level

    for layer_name, kind, mode, gdf in read_layers:
        n = len(gdf)
        total_features += n
        epsg = gdf.crs.to_epsg() if gdf.crs is not None else None
        crs_txt = f"EPSG:{epsg}" if epsg else (str(gdf.crs).split("\n")[0][:24] if gdf.crs else "none")
        crs_set.add(crs_txt)
        geom_types = sorted({g.geom_type for g in gdf.geometry if g is not None})
        notes = []

        if epsg != 4326:
            notes.append(("crit", f"CRS is {crs_txt}, expected EPSG:4326"))
            bump("fail")
        if n == 0:
            notes.append(("warn", "empty layer"))
            bump("warn")

        eid_dtype = eid_sample = None
        if kind == "edges":
            if "edge_id" not in gdf.columns:
                notes.append(("crit", "no edge_id column"))
                bump("fail")
            else:
                col = gdf["edge_id"]
                eid_dtype = str(col.dtype)
                eid_sample = None if not n else str(col.iloc[0])
                if col.dtype.kind == "f":               # float dtype == DBF rounded the 64-bit hash
                    notes.append(("crit", "edge_id is float — precision lost (ids corrupted)"))
                    bump("fail")
                elif col.dtype == object or eid_dtype in ("str", "string"):      # text: object, or pandas 3's str
                    notes.append(("ok", "edge_id exact (text — shapefile-safe)"))
                else:
                    notes.append(("ok", f"edge_id exact ({eid_dtype})"))
                if n and col.isna().any():
                    notes.append(("crit", "null edge_id(s)")); bump("fail")
                if n and col.astype(str).duplicated().any():
                    notes.append(("warn", "duplicate edge_id(s)")); bump("warn")

        # round-trip diff against the db
        roundtrip = None
        if kind == "edges" and mode in src_ids:
            exp = set(gdf["edge_id"].astype(str)) if "edge_id" in gdf.columns else set()
            src = src_ids[mode]
            missing, extra = len(src - exp), len(exp - src)
            roundtrip = {"src": len(src), "exp": len(exp), "missing": missing, "extra": extra}
            if missing or extra:
                notes.append(("crit", f"round-trip: {missing} missing, {extra} extra vs db"))
                bump("fail")
            else:
                notes.append(("ok", f"round-trip: all {len(src):,} ids match db"))

        qa_rows.append({
            "name": layer_name, "kind": kind, "mode": mode, "features": n,
            "geom": ", ".join(geom_types) or "—", "crs": crs_txt,
            "eid_dtype": eid_dtype, "eid_sample": eid_sample,
            "notes": [{"sev": s, "text": t} for s, t in notes], "roundtrip": roundtrip,
        })

        # geometry → quantized grid for the canvas
        if kind == "edges":
            bucket = modes_geo.setdefault(mode, {"edges": [], "nodes": []})
            hw = gdf["highway"] if "highway" in gdf.columns else [None] * n
            for geom, h in zip(gdf.geometry, hw):
                ci = _class_index(h)
                for coords in _iter_lines(geom):
                    flat = [ci]
                    for x, y in coords:
                        flat.append(qx(x)); flat.append(qy(y))
                    if len(flat) > 3:
                        bucket["edges"].append(flat)
        elif kind == "nodes":
            bucket = modes_geo.setdefault(mode, {"edges": [], "nodes": []})
            for geom in gdf.geometry:
                if geom is not None and not geom.is_empty and geom.geom_type == "Point":
                    bucket["nodes"].append(qx(geom.x)); bucket["nodes"].append(qy(geom.y))
        elif kind == "boundary":
            for geom in gdf.geometry:
                for ring in _iter_rings(geom):
                    flat = []
                    for x, y in ring:
                        flat.append(qx(x)); flat.append(qy(y))
                    if len(flat) > 4:
                        boundary_rings.append(flat)

    modes_present = sorted(k for k in modes_geo if any(v for v in modes_geo[k].values()))
    eid_summary = ("corrupted" if worst == "fail" and
                   any(any(nn["sev"] == "crit" and "edge_id" in nn["text"] for nn in r["notes"])
                       for r in qa_rows)
                   else "exact")
    rt_done = any(r["roundtrip"] for r in qa_rows)
    rt_ok = rt_done and all((r["roundtrip"]["missing"] == 0 and r["roundtrip"]["extra"] == 0)
                            for r in qa_rows if r["roundtrip"])

    return {
        "meta": {
            "file": Path(path).name, "format": fmt,
            "source_db": Path(source_db).name if source_db else None,
            "name": name or Path(path).stem,
        },
        "proj": {"minx": minx, "maxy": maxy, "kx": kx, "scale": scale},
        "grid": {"w": gridW, "h": gridH},
        "verdict": worst,
        "summary": {
            "layers": len(qa_rows), "features": total_features,
            "crs": (next(iter(crs_set)) if len(crs_set) == 1 else "mixed"),
            "edge_id": eid_summary,
            "roundtrip": ("pass" if rt_ok else "fail" if rt_done else "n/a"),
            "modes": modes_present,
        },
        "classes": [{"key": k, "label": lbl, "color": c, "width": w}
                    for (k, lbl, c, w) in _CLASSES],
        "layers": qa_rows,
        "geo": modes_geo,
        "boundary": boundary_rings,
    }


def render_html(payload, standalone=True):
    """Render the debug page. ``standalone`` → a full HTML document (for `duckosm gis-debug` / opening
    locally); otherwise the body-only content an Artifact wrapper expects."""
    data = json.dumps(payload, separators=(",", ":"))
    title = f"GIS export debug · {payload['meta']['file']}"
    content = _TEMPLATE.replace("/*__PAYLOAD__*/null", data).replace("__TITLE__", title)
    if not standalone:
        return f"<title>{title}</title>\n{content}"
    return (f"<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n"
            f"<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n"
            f"<title>{title}</title>\n</head>\n<body>\n{content}\n</body>\n</html>\n")


def write_debug(path, source_db=None, out=None, name=None, standalone=True, return_payload=False):
    """Build + write the debug page for an export. Returns the output path (and the page's data,
    with ``return_payload=True``: verdict, summary, per-layer checks)."""
    payload = build_payload(path, source_db=source_db, name=name)
    if out is None:
        out = f"{Path(path).stem}_gis_debug.html"
    Path(out).parent.mkdir(parents=True, exist_ok=True)
    Path(out).write_text(render_html(payload, standalone=standalone), encoding="utf-8")
    v = payload["verdict"].upper()
    logger.info(f"GIS debug [{v}]: {payload['summary']['layers']} layers, "
                f"{payload['summary']['features']:,} features -> {out}")
    return (out, payload) if return_payload else out


# ---------------------------------------------------------------------------------------------------
# The page (templates/gis_debug.html): CSS + markup + a tiny canvas renderer, all inlined. Theme-aware chrome
# (light/dark via tokens); the map itself is a committed dark "plotter" surface in both themes. `/*__PAYLOAD__*/null`
# is replaced with the JSON payload; `__TITLE__` with the document title. duckosm gis-debug reads the same file.
# ---------------------------------------------------------------------------------------------------
_TEMPLATE = (Path(__file__).parent / "templates" / "gis_debug.html").read_text(encoding="utf-8")
