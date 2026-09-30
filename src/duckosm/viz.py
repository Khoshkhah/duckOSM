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
BASEMAP_LAYERS = ["voyager", "positron", "dark_matter", "osm", "satellite", "blank"]


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
    # Extra tooltip fields, shown when the build carries them: lane count, speed, and the pedestrian/
    # cyclist functional class (walk_type on the walking graph, cycle_type on the cycling graph).
    info = [c for c in ("lanes", "maxspeed_kmh", "walk_type", "cycle_type") if c in edge_cols]
    # bridge/tunnel/layer drive roadstyle's grade-separation ordering (tunnels under, bridges over,
    # else the sign of the OSM layer tag). roadstyle reads these exact column names by default, so
    # passing them straight through is enough — they're carried onto every edge by the build.
    grade = [c for c in ("bridge", "tunnel", "layer") if c in edge_cols]
    # osm_id (the parent OSM way) and, when the build has it, the readable edge_ref are shown in the
    # hover tooltip. Both id-like fields are cast to VARCHAR so large ids survive JS Number precision.
    idcols = ["osm_id"] + (["edge_ref"] if "edge_ref" in edge_cols else [])
    select = ("CAST(edge_id AS VARCHAR) AS edge_id, CAST(osm_id AS VARCHAR) AS osm_id"
              + (", edge_ref" if "edge_ref" in edge_cols else "")
              + ", highway, COALESCE(name, '') AS name")
    select += "".join(f", {c}" for c in info)
    select += "".join(f", {c}" for c in grade)
    if arrows and "oneway" in edge_cols:
        select += ", oneway"
    select += ", ST_AsText(geometry) AS wkt"

    # Every road, incl. highway='service', lives in the single edges table.
    try:
        df = con.execute(f"SELECT {select} FROM {mode}.edges").df()
    except Exception as e:
        logger.warning(f"viz[{mode}]: cannot read edges ({e})")
        return None
    if df.empty:
        return None

    df["geometry"] = df["wkt"].map(_wkt.loads)
    g = add_level(gpd.GeoDataFrame(df.drop(columns=["wkt"]), geometry="geometry", crs="EPSG:4326"))

    # No color_by => roadstyle's classic OSM highway-class casing+fill.
    # basemaps=[...] adds the toggleable base-map layer switcher (chosen one first).
    # arrows=True => roadstyle overlays source->target direction chevrons; arrow_col="oneway"
    # restricts them to one-way edges (gray, ~2.8 m, shown at zoom >= 18 — roadstyle's defaults).
    layers = [basemap] + [b for b in BASEMAP_LAYERS if b != basemap]
    m = rs.render_edges(
        g, theme="light", basemap=basemap, basemaps=layers,
        tooltip=["edge_id", *idcols, "highway", "name", *info, "bridge", "tunnel", "level"],
        road_popup=["name", "edge_id", "edge_ref", "highway", "lanes", "bridge", "tunnel", "level"],
        copy_field="edge_id",
        name=f"{name} ({mode})", legend=True,
        # view_3d builds the extruded bridge decks the in-map 2D/3D toggle needs; pitch=0 still
        # opens the map flat.
        view_3d=True, pitch=0,
        arrows=arrows, arrow_col=("oneway" if arrows else None),
        boundary=(_boundary_geojson(con) if boundary else None),
    )
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    path = out / f"{name}_{mode}_network.html"
    m.save(str(path))
    logger.info(f"  Viz: {path}")
    return path


def add_level(g):
    """Add ``level``: the road's level as roadstyle draws it. A bridge is at least 1, a tunnel at
    most -1 (both carry their OSM ``layer`` beyond that), any other road 0 or its negative layer."""
    import pandas as pd

    def flag(col):
        if col not in g:
            return pd.Series(False, index=g.index)
        v = g[col].astype(str).str.lower()
        return g[col].notna() & ~v.isin(["", "no", "false", "0", "none", "nan"])

    ly = (pd.to_numeric(g["layer"], errors="coerce").fillna(0).astype(int) if "layer" in g
          else pd.Series(0, index=g.index))
    g["level"] = (ly.clip(upper=0).where(~flag("tunnel"), ly.clip(upper=-1))
                  .where(~flag("bridge"), ly.clip(lower=1)))
    return g


def _boundary_geojson(con):
    """Build a GeoJSON FeatureCollection of the clip/area boundary (`main.boundary`) for roadstyle to
    overlay as a dashed outline, so the map shows the extent the network was clipped to. Returns None
    if there's no boundary table (e.g. a whole-country PBF) — roadstyle then draws no overlay."""
    from shapely import wkt as _wkt
    from shapely.geometry import mapping

    try:
        rows = con.execute("SELECT ST_AsText(geom) FROM main.boundary WHERE geom IS NOT NULL").fetchall()
    except Exception:
        return None  # no boundary in this build (e.g. a whole-country PBF)
    feats = [{"type": "Feature", "properties": {}, "geometry": mapping(_wkt.loads(r[0]))}
             for r in rows if r[0]]
    return {"type": "FeatureCollection", "features": feats} if feats else None
