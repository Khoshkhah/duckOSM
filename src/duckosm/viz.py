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


def render_network(con, mode, name, basemap="voyager", out_dir="reports", arrows=False,
                   boundary=True):
    try:
        import geopandas as gpd
        from shapely import wkt as _wkt
        import roadstyle as rs
    except ImportError as e:
        logger.warning(f"viz needs geopandas + roadstyle ({e}) — skipping "
                       "(pip install geopandas roadstyle)")
        return None

    # Columns present on this build's edges (e.g. maxspeed_kmh needs process_speeds). edge_id is
    # a 64-bit hash > 2^53, cast to VARCHAR so it survives JS Number precision. lanes + speed are
    # added to the hover tooltip when present; oneway is pulled for the optional arrows.
    def _columns(table):
        return {r[0] for r in con.execute(
            "SELECT column_name FROM information_schema.columns "
            f"WHERE table_schema = '{mode}' AND table_name = '{table}'").fetchall()}

    edge_cols = _columns("edges")
    if not edge_cols:
        logger.warning(f"viz[{mode}]: no edges table to render")
        return None
    info = [c for c in ("lanes", "maxspeed_kmh") if c in edge_cols]      # extra tooltip fields
    select = "CAST(edge_id AS VARCHAR) AS edge_id, highway, COALESCE(name, '') AS name"
    select += "".join(f", {c}" for c in info)
    if arrows and "oneway" in edge_cols:
        select += ", oneway"
    select += ", ST_AsText(geometry) AS wkt"

    # Render EVERY road: the routing-graph edges PLUS the separated service roads (same styling),
    # so the map still shows the full network even though service edges live in their own table.
    froms = [f"{mode}.edges"] + ([f"{mode}.service_edges"] if _columns("service_edges") else [])
    try:
        df = con.execute(" UNION ALL ".join(f"SELECT {select} FROM {t}" for t in froms)).df()
    except Exception as e:
        logger.warning(f"viz[{mode}]: cannot read edges ({e})")
        return None
    if df.empty:
        return None

    df["geometry"] = df["wkt"].map(_wkt.loads)
    g = gpd.GeoDataFrame(df.drop(columns=["wkt"]), geometry="geometry", crs="EPSG:4326")

    # No color_by => roadstyle's classic OSM highway-class casing+fill.
    # basemaps=[...] adds the toggleable base-map layer switcher (chosen one first).
    # arrows=True => roadstyle overlays source->target direction chevrons; arrow_col="oneway"
    # restricts them to one-way edges (gray, ~2.8 m, shown at zoom >= 18 — roadstyle's defaults).
    layers = [basemap] + [b for b in BASEMAP_LAYERS if b != basemap]
    m = rs.render_edges(
        g, theme="light", basemap=basemap, basemaps=layers,
        tooltip=["edge_id", "highway", "name", *info], copy_field="edge_id",
        name=f"{name} ({mode})", legend=True,
        arrows=arrows, arrow_col=("oneway" if arrows else None),
    )
    if boundary:
        _add_boundary(con, m)
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    path = out / f"{name}_{mode}_network.html"
    m.save(str(path))
    logger.info(f"  Viz: {path}")
    return path


def _add_boundary(con, m):
    """Overlay the clip/area boundary (`main.boundary`) as a dashed outline, so the map shows
    the extent the network was clipped to. Skips silently if there's no boundary table."""
    import folium
    from shapely import wkt as _wkt
    from shapely.geometry import mapping

    try:
        rows = con.execute("SELECT ST_AsText(geom) FROM main.boundary WHERE geom IS NOT NULL").fetchall()
    except Exception:
        return  # no boundary in this build (e.g. a whole-country PBF)
    feats = [{"type": "Feature", "properties": {}, "geometry": mapping(_wkt.loads(r[0]))}
             for r in rows if r[0]]
    if not feats:
        return
    folium.GeoJson(
        {"type": "FeatureCollection", "features": feats}, name="boundary", control=False,
        style_function=lambda f: {"color": "#6a0dad", "weight": 2.5, "fill": False,
                                  "opacity": 0.9, "dashArray": "6 4"},
    ).add_to(m)
