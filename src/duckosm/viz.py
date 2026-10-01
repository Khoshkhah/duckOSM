"""Maps of a built network, drawn by mapstyle (docs/design/viz_on_mapstyle.md).

`duckosm viz` writes one page with every mode (``<name>_map.html``) and one page per mode
(``<name>_<mode>_network.html``), over the database's own base map (water, land, buildings, the
sea); private roads and bus lanes are drawn but never routed. The route planner is in
:mod:`duckosm.route_map`. Needs the ``viz`` extra (mapstyle).
"""
import logging
from pathlib import Path

logger = logging.getLogger("duckosm")


def mapstyle_module():
    """mapstyle, or an ImportError that says how to install it."""
    try:
        import mapstyle
    except ImportError as e:
        raise ImportError('maps need mapstyle: pip install "duckosm[viz]"') from e
    return mapstyle


def render_maps(db, modes=None, name=None, out_dir="reports", basemap=None, arrows=True,
                boundary=True, all_modes=True):
    """Write the maps of the built database ``db`` (a path) into ``out_dir``: with ``all_modes``,
    ``<name>_map.html`` with every mode, then ``<name>_<mode>_network.html`` for each of ``modes``
    (default: every mode in the db). ``basemap``: the first background (default: the database's own
    layers, blank). Returns the paths written."""
    import duckdb

    ms = mapstyle_module()
    con = duckdb.connect(str(db), read_only=True)
    con.execute("INSTALL spatial; LOAD spatial;")
    present = [r[0] for r in con.execute(
        "SELECT DISTINCT schema_name FROM duckdb_tables() WHERE table_name = 'edges' "
        "AND schema_name IN ('driving', 'walking', 'cycling') ORDER BY schema_name").fetchall()]
    outline = _boundary_geojson(con) if boundary else None
    con.close()
    modes = list(modes) if modes else present
    unknown = [m for m in modes if m not in present]
    if unknown:
        raise ValueError(f"mode(s) {unknown} not in {db} (present: {present or 'none'})")
    name = name or Path(db).stem
    kw = {"arrows": arrows}
    if basemap:
        kw["basemap"] = basemap
    if outline:
        kw["boundary"] = outline
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    pages = ([("all", out / f"{name}_map.html")] if all_modes else []) + \
            [(m, out / f"{name}_{m}_network.html") for m in modes]
    for mode, path in pages:
        ms.render_map(str(db), mode=mode, **kw).save(str(path))
        logger.info(f"  Map: {path}")
    return [p for _, p in pages]


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
