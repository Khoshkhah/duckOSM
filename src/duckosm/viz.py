"""
roadstyle network visualization → reports/<name>_<mode>_network.html.

Renders the built/clipped network with publication-quality cartography (casing + fill by
highway class, CARTO basemap, legend) using the `roadstyle` package. Optional: if
`geopandas` / `roadstyle` aren't installed it logs a warning and skips (never aborts a
build).
"""
import logging
from pathlib import Path

logger = logging.getLogger("duckosm")

# Base maps offered as a toggleable layer switcher in the output HTML.
BASEMAP_LAYERS = ["voyager", "positron", "esri_gray", "osm", "satellite"]


def render_network(con, mode, name, basemap="voyager", out_dir="reports", arrows=False):
    try:
        import geopandas as gpd
        from shapely import wkt as _wkt
        import roadstyle as rs
    except ImportError as e:
        logger.warning(f"viz needs geopandas + roadstyle ({e}) — skipping "
                       "(pip install geopandas roadstyle)")
        return None

    # edge_id is a 64-bit hash > 2^53 — cast to VARCHAR so it survives JS Number precision
    # (else the tooltip/copy round the low digits to a non-existent id). oneway/is_reverse are
    # pulled only for the optional direction-arrow overlay.
    cols = "CAST(edge_id AS VARCHAR) AS edge_id, highway, COALESCE(name, '') AS name"
    if arrows:
        cols += ", oneway, is_reverse"
    try:
        df = con.execute(
            f"SELECT {cols}, ST_AsText(geometry) AS wkt FROM {mode}.edges").df()
    except Exception as e:
        logger.warning(f"viz[{mode}]: cannot read edges ({e})")
        return None
    if df.empty:
        return None

    df["geometry"] = df["wkt"].map(_wkt.loads)
    drop = ["wkt"] + (["oneway", "is_reverse"] if arrows else [])
    g = gpd.GeoDataFrame(df.drop(columns=drop), geometry="geometry", crs="EPSG:4326")

    # No color_by => roadstyle's classic OSM highway-class casing+fill.
    # basemaps=[...] adds the toggleable base-map layer switcher (chosen one first).
    layers = [basemap] + [b for b in BASEMAP_LAYERS if b != basemap]
    m = rs.render_edges(
        g, theme="light", basemap=basemap, basemaps=layers,
        tooltip=["edge_id", "highway", "name"], copy_field="edge_id",
        name=f"{name} ({mode})", legend=True,
    )
    if arrows:
        _add_direction_arrows(m, df)
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    path = out / f"{name}_{mode}_network.html"
    m.save(str(path))
    logger.info(f"  Viz: {path}")
    return path


# Direction arrows are drawn ONLY on one-way edges (where direction matters and the orientation
# fix is visible). Each arrow is a small chevron (">") at the edge midpoint pointing
# source->target, emitted as plain GeoJSON line geometry in ONE layer — NOT folium's
# PolyLineTextPath, whose leaflet-textpath plugin freezes the browser at this many edges. Plain
# SVG paths are cheap (roadstyle already draws thousands). The layer is zoom-gated so it only
# renders once you zoom in to street level.
_ONEWAY_COLOR = "#555555"   # gray — one-way direction chevrons
_ARROW_SIZE = 0.000025      # chevron barb length in degrees (~2.8 m) — small, sits in-road


def _add_direction_arrows(m, df, min_zoom=18):
    """Overlay a chevron at each ONE-WAY edge's midpoint, pointing source->target."""
    import math
    import folium

    def _rot(vx, vy, deg):
        r = math.radians(deg)
        return vx * math.cos(r) - vy * math.sin(r), vx * math.sin(r) + vy * math.cos(r)

    feats = []
    for row in df.itertuples():
        if not bool(row.oneway):
            continue                                 # two-way roads omitted
        geom = getattr(row, "geometry", None)
        if geom is None or geom.geom_type != "LineString" or geom.length == 0:
            continue
        mid = geom.interpolate(0.5, normalized=True)
        a = geom.interpolate(0.49, normalized=True)
        b = geom.interpolate(0.51, normalized=True)
        cs = math.cos(math.radians(mid.y)) or 1e-6   # lat-correction so the chevron isn't skewed
        dx, dy = (b.x - a.x) * cs, (b.y - a.y)
        dn = math.hypot(dx, dy)
        if dn == 0:
            continue
        ux, uy = dx / dn, dy / dn                    # unit travel direction (equal-aspect frame)
        tx, ty = mid.x * cs, mid.y                   # tip at midpoint
        (b1x, b1y), (b2x, b2y) = _rot(-ux, -uy, 35), _rot(-ux, -uy, -35)   # barbs trailing the tip
        to_ll = lambda px, py: [px / cs, py]         # back to lon/lat
        feats.append({"type": "Feature", "properties": {}, "geometry": {"type": "LineString",
            "coordinates": [to_ll(tx + b1x * _ARROW_SIZE, ty + b1y * _ARROW_SIZE),
                            to_ll(tx, ty),
                            to_ll(tx + b2x * _ARROW_SIZE, ty + b2y * _ARROW_SIZE)]}})
    if not feats:
        return
    gj = folium.GeoJson(
        {"type": "FeatureCollection", "features": feats}, name="one-way direction",
        style_function=lambda f: {"color": _ONEWAY_COLOR, "weight": 2, "opacity": 0.9},
    )
    gj.add_to(m)
    _show_layer_above_zoom(m, gj, min_zoom=min_zoom)
    logger.info(f"  direction arrows: {len(feats)} one-way chevron(s), shown at zoom >= {min_zoom}")


def _show_layer_above_zoom(m, layer, min_zoom):
    """Bind `layer` visibility to map zoom: render it only at zoom >= min_zoom, so the
    zoomed-out overview stays clean and the arrow glyphs aren't drawn until they're useful."""
    from branca.element import MacroElement
    from jinja2 import Template

    el = MacroElement()
    el._template = Template(
        "{% macro script(this, kwargs) %}\n"
        "  var _amap = {{ this._parent.get_name() }};\n"
        "  var _alyr = " + layer.get_name() + ";\n"
        "  function _arrowVis() {\n"
        "    if (_amap.getZoom() >= " + str(min_zoom) + ") {\n"
        "      if (!_amap.hasLayer(_alyr)) _amap.addLayer(_alyr);\n"
        "    } else if (_amap.hasLayer(_alyr)) { _amap.removeLayer(_alyr); }\n"
        "  }\n"
        "  _amap.on('zoomend', _arrowVis); _arrowVis();\n"
        "{% endmacro %}"
    )
    m.add_child(el)
