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
        con.execute("INSTALL spatial; LOAD spatial;")
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
# The page. CSS + markup + a tiny canvas renderer, all inlined. Theme-aware chrome (light/dark via
# tokens); the map itself is a committed dark "plotter" surface in both themes. `/*__PAYLOAD__*/null`
# is replaced with the JSON payload; `__TITLE__` with the document title.
# ---------------------------------------------------------------------------------------------------
_TEMPLATE = r"""<style>
:root{
  --bg:#eef1f4; --panel:#ffffff; --panel-2:#f6f8fa; --ink:#0f1720; --muted:#5b6b7a;
  --line:#dce2e8; --accent:#0d7d8c; --accent-ink:#0a5b66;
  --ok:#1a7f37; --warn:#9a6700; --crit:#cf222e;
  --map-bg:#0b1017; --map-grid:rgba(255,255,255,.05); --map-frame:#1c2530;
  --mono:ui-monospace,"SF Mono","Cascadia Code",Menlo,Consolas,monospace;
  --sans:system-ui,-apple-system,"Segoe UI",Roboto,Helvetica,Arial,sans-serif;
}
@media (prefers-color-scheme:dark){
  :root{
    --bg:#0c1116; --panel:#121a22; --panel-2:#0e151c; --ink:#e6edf3; --muted:#8b9aa8;
    --line:#222c37; --accent:#35c4d4; --accent-ink:#7fe3ee;
    --ok:#3fb950; --warn:#d29922; --crit:#f85149;
  }
}
:root[data-theme="light"]{
  --bg:#eef1f4; --panel:#ffffff; --panel-2:#f6f8fa; --ink:#0f1720; --muted:#5b6b7a;
  --line:#dce2e8; --accent:#0d7d8c; --accent-ink:#0a5b66;
  --ok:#1a7f37; --warn:#9a6700; --crit:#cf222e;
}
:root[data-theme="dark"]{
  --bg:#0c1116; --panel:#121a22; --panel-2:#0e151c; --ink:#e6edf3; --muted:#8b9aa8;
  --line:#222c37; --accent:#35c4d4; --accent-ink:#7fe3ee;
  --ok:#3fb950; --warn:#d29922; --crit:#f85149;
}
*{box-sizing:border-box}
.gxd{font-family:var(--sans);color:var(--ink);background:var(--bg);min-height:100vh;
  padding:20px;display:flex;flex-direction:column;gap:16px;line-height:1.45}
.gxd h1,.gxd h2,.gxd p{margin:0}
.eyebrow{font-size:11px;letter-spacing:.14em;text-transform:uppercase;color:var(--muted);font-weight:600}
.gxd-head{display:flex;flex-wrap:wrap;align-items:flex-end;gap:14px 20px;justify-content:space-between}
.gxd-title h1{font-size:20px;font-weight:650;letter-spacing:-.01em;text-wrap:balance}
.gxd-file{font-family:var(--mono);font-size:13px;color:var(--muted);margin-top:3px}
.gxd-file b{color:var(--ink);font-weight:600}
.verdict{display:inline-flex;align-items:center;gap:8px;padding:7px 14px;border-radius:999px;
  font-weight:650;font-size:13px;letter-spacing:.02em;border:1px solid transparent}
.verdict .dot{width:9px;height:9px;border-radius:50%}
.v-pass{background:color-mix(in srgb,var(--ok) 14%,transparent);color:var(--ok);border-color:color-mix(in srgb,var(--ok) 35%,transparent)}
.v-pass .dot{background:var(--ok)}
.v-warn{background:color-mix(in srgb,var(--warn) 16%,transparent);color:var(--warn);border-color:color-mix(in srgb,var(--warn) 38%,transparent)}
.v-warn .dot{background:var(--warn)}
.v-fail{background:color-mix(in srgb,var(--crit) 15%,transparent);color:var(--crit);border-color:color-mix(in srgb,var(--crit) 40%,transparent)}
.v-fail .dot{background:var(--crit)}
.tiles{display:grid;grid-template-columns:repeat(auto-fit,minmax(140px,1fr));gap:10px}
.tile{background:var(--panel);border:1px solid var(--line);border-radius:12px;padding:12px 14px;
  display:flex;flex-direction:column;gap:5px}
.tile .k{font-size:11px;letter-spacing:.08em;text-transform:uppercase;color:var(--muted);font-weight:600}
.tile .v{font-family:var(--mono);font-size:20px;font-weight:600;font-variant-numeric:tabular-nums}
.tile .v.sm{font-size:15px}
.tile .v.ok{color:var(--ok)} .tile .v.warn{color:var(--warn)} .tile .v.crit{color:var(--crit)}
.body{display:grid;grid-template-columns:1fr 380px;gap:16px;align-items:stretch;min-height:0}
@media (max-width:900px){.body{grid-template-columns:1fr}}
.mapwrap{position:relative;background:var(--map-bg);border:1px solid var(--map-frame);border-radius:14px;
  overflow:hidden;min-height:460px;display:flex}
.mapwrap canvas{width:100%;height:100%;display:block;cursor:grab;touch-action:none}
.mapwrap canvas:active{cursor:grabbing}
.map-ui{position:absolute;left:12px;top:12px;display:flex;flex-direction:column;gap:8px;
  max-width:calc(100% - 24px)}
.seg{display:inline-flex;background:rgba(10,15,22,.72);border:1px solid rgba(255,255,255,.12);
  border-radius:9px;padding:3px;backdrop-filter:blur(6px);flex-wrap:wrap}
.seg button{font-family:var(--sans);font-size:12px;font-weight:600;color:#c4d0da;background:transparent;
  border:0;padding:5px 11px;border-radius:6px;cursor:pointer;letter-spacing:.01em}
.seg button:hover{color:#fff}
.seg button[aria-pressed="true"]{background:var(--accent);color:#04222a}
.seg button:focus-visible{outline:2px solid var(--accent-ink);outline-offset:1px}
.map-foot{position:absolute;right:12px;bottom:11px;display:flex;gap:10px;align-items:center;
  font-family:var(--mono);font-size:11px;color:#8ea0ad;background:rgba(10,15,22,.66);
  border:1px solid rgba(255,255,255,.1);border-radius:8px;padding:5px 9px;backdrop-filter:blur(6px)}
.map-foot b{color:#d7e2ea;font-weight:600}
.legend{position:absolute;right:12px;top:12px;background:rgba(10,15,22,.72);
  border:1px solid rgba(255,255,255,.12);border-radius:10px;padding:9px 11px;backdrop-filter:blur(6px);
  display:flex;flex-direction:column;gap:5px;max-height:60%;overflow:auto}
.legend .lg-h{font-size:10px;letter-spacing:.12em;text-transform:uppercase;color:#8ea0ad;font-weight:700;margin-bottom:2px}
.legend .row{display:flex;align-items:center;gap:8px;font-size:11.5px;color:#c9d5de}
.legend .sw{width:16px;height:3px;border-radius:2px;flex:none}
.legend .sw.dot{width:8px;height:8px;border-radius:50%}
.legend .sw.dash{height:0;border-top:2px dashed #ff6ec7;background:none;border-radius:0}
.side{display:flex;flex-direction:column;gap:12px;min-height:0}
.card{background:var(--panel);border:1px solid var(--line);border-radius:14px;padding:14px 15px}
.card h2{font-size:12px;letter-spacing:.1em;text-transform:uppercase;color:var(--muted);
  font-weight:700;margin-bottom:10px}
.layer{border:1px solid var(--line);border-radius:11px;padding:10px 12px;margin-bottom:9px;background:var(--panel-2)}
.layer:last-child{margin-bottom:0}
.layer .lh{display:flex;align-items:center;justify-content:space-between;gap:8px}
.layer .ln{font-family:var(--mono);font-size:13px;font-weight:600}
.layer .cnt{font-family:var(--mono);font-size:12px;color:var(--muted);font-variant-numeric:tabular-nums}
.layer .meta{display:flex;flex-wrap:wrap;gap:6px 14px;margin-top:6px;font-family:var(--mono);
  font-size:11px;color:var(--muted)}
.layer .meta b{color:var(--ink);font-weight:600}
.notes{list-style:none;padding:0;margin:8px 0 0;display:flex;flex-direction:column;gap:4px}
.notes li{display:flex;gap:7px;align-items:flex-start;font-size:12px}
.chip{flex:none;margin-top:1px;width:7px;height:7px;border-radius:50%}
.chip.ok{background:var(--ok)} .chip.warn{background:var(--warn)} .chip.crit{background:var(--crit)}
.notes li.ok{color:var(--muted)} .notes li.warn{color:var(--warn)} .notes li.crit{color:var(--crit)}
.foot{font-size:11.5px;color:var(--muted);font-family:var(--mono)}
.foot code{color:var(--ink)}
@media (prefers-reduced-motion:reduce){*{transition:none!important}}
</style>

<div class="gxd" id="gxd">
  <header class="gxd-head">
    <div class="gxd-title">
      <div class="eyebrow">GIS export · debug</div>
      <h1>Exported network — read back through GDAL</h1>
      <div class="gxd-file">reading <b id="mfile">—</b> · <span id="mfmt">—</span><span id="msrc"></span></div>
    </div>
    <div id="verdict" class="verdict v-pass"><span class="dot"></span><span id="verdict-t">PASS</span></div>
  </header>

  <section class="tiles" id="tiles"></section>

  <section class="body">
    <div class="mapwrap">
      <canvas id="map"></canvas>
      <div class="map-ui">
        <div class="seg" id="modeseg" role="group" aria-label="Mode"></div>
        <div class="seg" id="layerseg" role="group" aria-label="Layers">
          <button data-lyr="edges" aria-pressed="true">Edges</button>
          <button data-lyr="nodes" aria-pressed="false">Nodes</button>
          <button data-lyr="boundary" aria-pressed="true">Boundary</button>
          <button data-lyr="reset" aria-pressed="false" title="Reset view">Reset</button>
        </div>
      </div>
      <div class="legend" id="legend"></div>
      <div class="map-foot"><span id="cur">—</span> · <b id="zoom">1.0×</b></div>
    </div>

    <aside class="side">
      <div class="card" style="flex:1;overflow:auto;min-height:0">
        <h2>Layer audit</h2>
        <div id="layers"></div>
      </div>
      <div class="card foot">
        Reads the exported file with GDAL — the same path QGIS / ArcGIS / FME take — so this verifies
        the file on disk, not the duckOSM database. Geometry drawn client-side, no tiles.
      </div>
    </aside>
  </section>
</div>

<script>
const DATA = /*__PAYLOAD__*/null;
(function(){
  const $=id=>document.getElementById(id);
  const fmtN=n=>n.toLocaleString('en-US');

  // ---- header + tiles ---------------------------------------------------------------------------
  $('mfile').textContent=DATA.meta.file;
  $('mfmt').textContent=DATA.meta.format;
  $('msrc').textContent=DATA.meta.source_db?(' · vs '+DATA.meta.source_db):'';
  const V={pass:['v-pass','PASS'],warn:['v-warn','CHECK'],fail:['v-fail','FAIL']}[DATA.verdict];
  $('verdict').className='verdict '+V[0]; $('verdict-t').textContent=V[1];

  const S=DATA.summary;
  const rtCls=S.roundtrip==='pass'?'ok':S.roundtrip==='fail'?'crit':'';
  const eidCls=S.edge_id==='exact'?'ok':'crit';
  const tiles=[
    ['Layers',fmtN(S.layers),''],
    ['Features',fmtN(S.features),''],
    ['CRS',S.crs,S.crs==='EPSG:4326'?'ok sm':'crit sm'],
    ['edge_id',S.edge_id,eidCls+' sm'],
    ['Round-trip',S.roundtrip==='n/a'?'not run':S.roundtrip,rtCls+' sm'],
    ['Modes',S.modes.join(' · ')||'—','sm'],
  ];
  $('tiles').innerHTML=tiles.map(([k,v,c])=>
    `<div class="tile"><span class="k">${k}</span><span class="v ${c}">${v}</span></div>`).join('');

  // ---- layer audit ------------------------------------------------------------------------------
  $('layers').innerHTML=DATA.layers.map(L=>{
    const meta=[`<span>geom <b>${L.geom}</b></span>`,`<span>crs <b>${L.crs}</b></span>`];
    if(L.eid_dtype)meta.push(`<span>edge_id <b>${L.eid_dtype}</b></span>`);
    if(L.eid_sample)meta.push(`<span>e.g. <b>${L.eid_sample}</b></span>`);
    const notes=L.notes.map(n=>`<li class="${n.sev}"><span class="chip ${n.sev}"></span>${n.text}</li>`).join('');
    return `<div class="layer"><div class="lh"><span class="ln">${L.name}</span>
      <span class="cnt">${fmtN(L.features)} feat</span></div>
      <div class="meta">${meta.join('')}</div>
      ${notes?`<ul class="notes">${notes}</ul>`:''}</div>`;
  }).join('');

  // ---- legend -----------------------------------------------------------------------------------
  const usedClasses=new Set();
  for(const m in DATA.geo)for(const e of DATA.geo[m].edges)usedClasses.add(e[0]);
  let lg='<div class="lg-h">Highway class</div>';
  DATA.classes.forEach((c,i)=>{ if(usedClasses.has(i))
    lg+=`<div class="row"><span class="sw" style="background:${c.color}"></span>${c.label}</div>`; });
  lg+='<div class="row" style="margin-top:5px"><span class="sw dot" style="background:'+
      getComputedStyle(document.documentElement).getPropertyValue('--accent')+'"></span>Nodes</div>';
  if(DATA.boundary.length)lg+='<div class="row"><span class="sw dash"></span>Boundary</div>';
  $('legend').innerHTML=lg;

  // ---- mode toggles -----------------------------------------------------------------------------
  const modes=DATA.summary.modes;
  let curMode='ALL';
  const show={edges:true,nodes:false,boundary:true};
  const mseg=$('modeseg');
  const mkBtn=(label,val,pressed)=>{const b=document.createElement('button');b.textContent=label;
    b.dataset.mode=val;b.setAttribute('aria-pressed',pressed);return b;};
  if(modes.length>1)mseg.appendChild(mkBtn('All','ALL',true));
  modes.forEach(m=>mseg.appendChild(mkBtn(m,m,modes.length===1)));
  if(modes.length===1)curMode=modes[0];
  mseg.addEventListener('click',e=>{const b=e.target.closest('button');if(!b)return;
    curMode=b.dataset.mode;[...mseg.children].forEach(x=>x.setAttribute('aria-pressed',x===b));draw();});

  $('layerseg').addEventListener('click',e=>{const b=e.target.closest('button');if(!b)return;
    const l=b.dataset.lyr;
    if(l==='reset'){fit();return;}
    show[l]=!show[l];b.setAttribute('aria-pressed',show[l]);draw();});

  // ---- canvas renderer --------------------------------------------------------------------------
  const cv=$('map'),ctx=cv.getContext('2d');
  const GW=DATA.grid.w,GH=DATA.grid.h,P=DATA.proj,CLS=DATA.classes;
  let view={s:1,tx:0,ty:0},dpr=Math.min(window.devicePixelRatio||1,2);

  function resize(){const r=cv.getBoundingClientRect();cv.width=Math.max(1,r.width*dpr);
    cv.height=Math.max(1,r.height*dpr);fit();}
  function fit(){const r=cv.getBoundingClientRect();const pad=24;
    const s=Math.min((r.width-2*pad)/GW,(r.height-2*pad)/GH);
    view.s=s;view.tx=(r.width-GW*s)/2;view.ty=(r.height-GH*s)/2;draw();}
  const X=gx=>(gx*view.s+view.tx)*dpr, Y=gy=>(gy*view.s+view.ty)*dpr;

  function modesToDraw(){return curMode==='ALL'?modes:[curMode];}

  function draw(){
    ctx.setTransform(1,0,0,1,0,0);
    ctx.fillStyle=getComputedStyle(document.documentElement).getPropertyValue('--map-bg')||'#0b1017';
    ctx.fillRect(0,0,cv.width,cv.height);
    // subtle frame of the data extent
    ctx.strokeStyle='rgba(255,255,255,.06)';ctx.lineWidth=1;
    ctx.strokeRect(X(0),Y(0),GW*view.s*dpr,GH*view.s*dpr);

    const dl=modesToDraw();
    if(show.edges){
      // bucket by class so strokeStyle/width is set once per class (z-order = class order)
      const byClass=CLS.map(()=>[]);
      for(const m of dl){const g=DATA.geo[m];if(!g)continue;
        for(const e of g.edges)byClass[e[0]].push(e);}
      ctx.lineCap='round';ctx.lineJoin='round';
      byClass.forEach((arr,ci)=>{if(!arr.length)return;
        ctx.strokeStyle=CLS[ci].color;
        ctx.lineWidth=Math.max(.6,CLS[ci].width*Math.min(1.7,Math.max(.5,view.s*GW/2600)))*dpr;
        ctx.beginPath();
        for(const e of arr){ctx.moveTo(X(e[1]),Y(e[2]));
          for(let i=3;i<e.length;i+=2)ctx.lineTo(X(e[i]),Y(e[i+1]));}
        ctx.stroke();});
    }
    if(show.boundary&&DATA.boundary.length){
      ctx.strokeStyle='#ff6ec7';ctx.lineWidth=1.6*dpr;ctx.setLineDash([7*dpr,5*dpr]);
      ctx.beginPath();
      for(const r of DATA.boundary){ctx.moveTo(X(r[0]),Y(r[1]));
        for(let i=2;i<r.length;i+=2)ctx.lineTo(X(r[i]),Y(r[i+1]));}
      ctx.stroke();ctx.setLineDash([]);
    }
    if(show.nodes){
      ctx.fillStyle=getComputedStyle(document.documentElement).getPropertyValue('--accent')||'#35c4d4';
      const rad=Math.max(.7,Math.min(2.6,view.s*GW/2200))*dpr;
      for(const m of dl){const g=DATA.geo[m];if(!g)continue;
        for(let i=0;i<g.nodes.length;i+=2){
          ctx.beginPath();ctx.arc(X(g.nodes[i]),Y(g.nodes[i+1]),rad,0,6.2832);ctx.fill();}}
    }
    $('zoom').textContent=view.s.toFixed(2).replace(/0$/,'')+'×';
  }

  // pan / zoom
  let drag=null;
  cv.addEventListener('pointerdown',e=>{drag={x:e.clientX,y:e.clientY,tx:view.tx,ty:view.ty};
    cv.setPointerCapture(e.pointerId);});
  cv.addEventListener('pointermove',e=>{
    const r=cv.getBoundingClientRect();
    const gx=(e.clientX-r.left-view.tx)/view.s, gy=(e.clientY-r.top-view.ty)/view.s;
    const lon=P.minx+(gx/P.scale)/P.kx, lat=P.maxy-(gy/P.scale);
    if(gx>=0&&gx<=GW&&gy>=0&&gy<=GH)$('cur').textContent=lat.toFixed(5)+', '+lon.toFixed(5);
    if(drag){view.tx=drag.tx+(e.clientX-drag.x);view.ty=drag.ty+(e.clientY-drag.y);draw();}
  });
  const endDrag=()=>drag=null;
  cv.addEventListener('pointerup',endDrag);cv.addEventListener('pointercancel',endDrag);
  cv.addEventListener('wheel',e=>{e.preventDefault();
    const r=cv.getBoundingClientRect(),mx=e.clientX-r.left,my=e.clientY-r.top;
    const k=Math.exp(-e.deltaY*0.0016),ns=Math.min(400,Math.max(.05,view.s*k));
    view.tx=mx-(mx-view.tx)*(ns/view.s);view.ty=my-(my-view.ty)*(ns/view.s);view.s=ns;draw();
  },{passive:false});

  new ResizeObserver(resize).observe(cv);
  resize();
})();
</script>"""
