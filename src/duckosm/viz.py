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


def render_network(con, mode, name, basemap="voyager", out_dir="reports", arrows=True,
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

    # Every road, incl. highway='service', lives in the single edges table; roads you may not use
    # (private_edges: private, or for buses only; never routable) are drawn too, in their own colour
    # (docs/design/access_private.md, bus_only_edges.md). `access` says which; NULL = routable.
    has_private = con.execute("SELECT count(*) FROM information_schema.tables WHERE table_schema = ? "
                              "AND table_name = 'private_edges'", [mode]).fetchone()[0] > 0
    sql = f"SELECT {select}, NULL AS access FROM {mode}.edges"
    if has_private:
        sql += f" UNION ALL SELECT {select}, access FROM {mode}.private_edges"
    try:
        df = con.execute(sql).df()
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
        tooltip=["edge_id", *idcols, "highway", "name", *info, "access"], copy_field="edge_id",
        road_popup=["name", "edge_id", "edge_ref", "highway", "lanes", "bridge", "tunnel", "access"],
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
    html = m.html
    if df["access"].notna().any():                       # roads you may not use: own colour + toggle
        html = html.replace("</body>", restricted_roads_js() + "</body>", 1)
    path.write_text(html, encoding="utf-8")
    logger.info(f"  Viz: {path}")
    return path


# Roads you may not use (private_edges: private roads, and in driving bus-only roads and bus lanes)
# on a viz / route map: painted in their own colour on every road layer (again after each rsColor,
# which rebuilds the road colours), and a row each in roadstyle's roads box (after Bridges / Tunnels),
# "Private roads" and "Bus lanes", that hides and shows them. Read from the `access` property.
_RESTRICTED_ROADS_JS = """<script>
(function () {
  var KINDS = [["private", "__PRIVATE__", " Private roads", "dk-flt-private"],
               ["bus", "__BUS__", " Bus lanes", "dk-flt-bus"]];
  var is = function (v) { return ["==", ["get", "access"], v]; };
  function paint() {
    map.getStyle().layers.forEach(function (l) {
      if (l.type !== "line" || !/^roads-.*fill/.test(l.id)) return;
      var c = map.getPaintProperty(l.id, "line-color");
      if (c[0] === "case" && JSON.stringify(c[1]) === JSON.stringify(is("private"))) return;   // painted already
      map.setPaintProperty(l.id, "line-color", ["case", is("private"), KINDS[0][1], is("bus"), KINDS[1][1], c]);
    });
  }
  (function init() {
    var body = document.querySelector(".flt-body");
    if (!(window.map && window.rsQuery && map.isStyleLoaded() && body)) return setTimeout(init, 200);
    var ids = {}, hidden = {}, all = rsQuery(function () { return true; });
    KINDS.forEach(function (k) { ids[k[0]] = rsQuery(function (p) { return p.access === k[0]; }); });
    if (!ids.private.length && !ids.bus.length) return;
    paint();
    document.addEventListener("rs:colorchange", paint);
    function apply() {
      var off = {};
      KINDS.forEach(function (k) { if (hidden[k[0]]) ids[k[0]].forEach(function (i) { off[i] = 1; }); });
      rsFilter(Object.keys(off).length ? all.filter(function (i) { return !off[i]; }) : null);
    }
    var first = !body.querySelector(".flt-grade");
    KINDS.forEach(function (k) {
      if (!ids[k[0]].length) return;
      var lab = document.createElement("label"), cb = document.createElement("input"), sw = document.createElement("span");
      if (first) { lab.style.cssText = "margin-top:4px;padding-top:4px;border-top:1px solid #ddd"; first = false; }
      cb.type = "checkbox"; cb.checked = true; cb.id = k[3];
      cb.onchange = function () { hidden[k[0]] = !cb.checked; apply(); };
      sw.className = "flt-sw"; sw.style.background = k[1];
      lab.appendChild(cb); lab.appendChild(sw); lab.appendChild(document.createTextNode(k[2]));
      body.appendChild(lab);
    });
  })();
})();
</script>
"""


def restricted_roads_js(private="#c8c8c8", bus="#9db8d9"):
    """The <script> for roads you may not use (see above): private ones in ``private``, bus-only
    ones in ``bus``."""
    return _RESTRICTED_ROADS_JS.replace("__PRIVATE__", private).replace("__BUS__", bus)


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
