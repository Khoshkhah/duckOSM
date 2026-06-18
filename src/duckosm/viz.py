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
    # (else the tooltip/copy round the low digits to a non-existent id). `oneway` is pulled only
    # for the optional direction arrows (roadstyle draws a chevron per one-way edge).
    cols = "CAST(edge_id AS VARCHAR) AS edge_id, highway, COALESCE(name, '') AS name"
    if arrows:
        cols += ", oneway"
    try:
        df = con.execute(
            f"SELECT {cols}, ST_AsText(geometry) AS wkt FROM {mode}.edges").df()
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
        tooltip=["edge_id", "highway", "name"], copy_field="edge_id",
        name=f"{name} ({mode})", legend=True,
        arrows=arrows, arrow_col=("oneway" if arrows else None),
    )
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    path = out / f"{name}_{mode}_network.html"
    m.save(str(path))
    logger.info(f"  Viz: {path}")
    return path
