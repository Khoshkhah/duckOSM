#!/usr/bin/env python
"""Render a computed **multimodal** route (walk → drive → walk / park-and-ride) on a map,
coloured **by transport mode**, using the sibling `mapstyle` (deck.gl) viewer.

This is the visual counterpart to `tests/test_multimodal.py`: it builds the intermodal `mm.*`
graph (if absent), routes across it with `duckosm.route_multimodal`, and draws each leg in its
mode colour — walking green, driving red, cycling blue — over an OSM basemap.

    # auto-pick a walk->drive->walk demo trip across Sodermalm and render it
    python scripts/multimodal_route_map.py --db data/db/sodermalm_pbf.duckdb
    # a specific trip between two OSM junction node_ids
    python scripts/multimodal_route_map.py --db data/db/sodermalm_pbf.duckdb --src 336296072 --dst 35120411
    python scripts/multimodal_route_map.py --db net.duckdb --out-dir reports/mm --basemap satellite

`mapstyle` renders a **directory** (index.html + data/*.geojson) that must be served over HTTP —
the script writes a `serve.py` next to it; run `python <out-dir>/serve.py` and open the printed URL.

Needs: geopandas + shapely (in the duckOSM venv) and the `mapstyle` package. mapstyle is a sibling
checkout, not installed in this venv — the script finds it at ../mapstyle/src automatically (override
with --mapstyle-src or the MAPSTYLE_SRC env var). Its runtime deps (roadstyle/geopandas/shapely/
folium/pyyaml) are already present here.
"""
import argparse
import os
import sys
from pathlib import Path

import duckdb

from duckosm import route_multimodal
from duckosm.processors import MultimodalBuilder

# walking green, driving red, cycling blue — matches mapstyle's own per-mode palette (COMBO_COLOR).
MODE_COLOR = {"walking": "#27ae60", "driving": "#e74c3c", "cycling": "#2b6cb0"}
# draw vehicular legs under the walking legs so the short walk ends stay visible on top.
MODE_Z = {"driving": 1, "cycling": 1, "walking": 2}


def _import_mapstyle(mapstyle_src=None):
    """Import `mapstyle`, adding its sibling `src/` to sys.path if it isn't installed in this venv."""
    try:
        import mapstyle  # noqa: F401
        return
    except ImportError:
        pass
    candidates = [mapstyle_src, os.environ.get("MAPSTYLE_SRC"),
                  str(Path(__file__).resolve().parents[2] / "mapstyle" / "src")]
    for c in candidates:
        if c and Path(c).exists():
            sys.path.insert(0, c)
            try:
                import mapstyle  # noqa: F401
                return
            except ImportError:
                continue
    raise SystemExit(
        "could not import `mapstyle`. Point --mapstyle-src (or $MAPSTYLE_SRC) at the sibling "
        "mapstyle checkout's src/ dir, e.g. --mapstyle-src ../mapstyle/src")


def _ensure_mm(con, transfer_cost):
    """Build the mm.* graph if this db doesn't already have one."""
    has_mm = con.execute(
        "SELECT COUNT(*) FROM duckdb_tables() WHERE schema_name = 'mm' "
        "AND table_name = 'edges'").fetchone()[0]
    if not has_mm:
        print("building mm.* graph (none present) ...")
        MultimodalBuilder(con, transfer_s=transfer_cost).run()


def _autopick(con, start_mode="walking", end_mode="walking"):
    """Pick a demo trip: far-apart *walking-only* junctions, preferring a walk→drive→walk route
    (so both ends actually have a walking leg). Returns (src, dst)."""
    df = con.execute("""
        WITH wonly AS (SELECT node_id FROM walking.nodes EXCEPT SELECT node_id FROM driving.nodes)
        SELECT n.node_id, ST_X(n.geom) AS x, ST_Y(n.geom) AS y
        FROM walking.nodes n JOIN wonly w USING (node_id)""").df()
    if df.empty:
        raise SystemExit("no walking-only nodes to auto-pick from — pass --src / --dst")
    ext = {"W": df.loc[df.x.idxmin()], "E": df.loc[df.x.idxmax()],
           "S": df.loc[df.y.idxmin()], "N": df.loc[df.y.idxmax()]}
    fallback = None
    for a, b in [("W", "E"), ("S", "N"), ("W", "N"), ("E", "S")]:
        src, dst = int(ext[a].node_id), int(ext[b].node_id)
        r = route_multimodal(con, src, dst, start_mode=start_mode, end_mode=end_mode)
        if r is None:
            continue
        modes = [leg["mode"] for leg in r["legs"]]
        if fallback is None:
            fallback = (src, dst)
        if modes and modes[0] == "walking" and modes[-1] == "walking" and any(
                m != "walking" for m in modes):
            return src, dst
    if fallback is None:
        raise SystemExit("auto-pick found no route — pass --src / --dst")
    return fallback


def _mode_layers(route, Layer):
    """One mapstyle line `Layer` per mode, coloured by mode, built from the route's leg geometries."""
    import geopandas as gpd
    from shapely import wkt as _wkt

    by_mode = {}  # mode -> {"geometry": [...], "highway": [...]}
    for leg in route["legs"]:
        m = leg["mode"]
        d = by_mode.setdefault(m, {"geometry": [], "highway": []})
        for p in leg["path"]:
            if p.get("geometry"):
                d["geometry"].append(_wkt.loads(p["geometry"]))
                d["highway"].append(p.get("highway") or "residential")
    layers = []
    for m, d in by_mode.items():
        if not d["geometry"]:
            continue
        gdf = gpd.GeoDataFrame({"highway": d["highway"]},
                               geometry=d["geometry"], crs="EPSG:4326")
        layers.append(Layer(f"route_{m}", gdf, "line",
                            color=MODE_COLOR.get(m, "#999999"), z=MODE_Z.get(m, 2)))
    # bottom -> top: vehicular first, walking on top
    layers.sort(key=lambda ly: ly.z or 0)
    return layers


def render_multimodal_route(db, src=None, dst=None, out_dir=None, transfer_cost=60.0,
                            basemap="osm", start_mode="walking", end_mode="walking",
                            mapstyle_src=None):
    """Route across the mm.* graph of `db` and render it by mode with mapstyle. Returns the
    written index.html path. Builds mm.* in `db` if absent (so pass a scratch copy if you don't
    want to modify the source db)."""
    _import_mapstyle(mapstyle_src)
    from mapstyle import Layer, render_web
    try:
        from mapstyle.render_web import write_serve
    except ImportError:
        write_serve = None

    db = Path(db)
    out_dir = Path(out_dir) if out_dir else Path("reports") / f"{db.stem}_multimodal_route"

    con = duckdb.connect(str(db))                    # read-write: may build mm.*
    con.execute("INSTALL spatial; LOAD spatial;")
    _ensure_mm(con, transfer_cost)

    if src is None or dst is None:
        src, dst = _autopick(con, start_mode, end_mode)
        print(f"auto-picked demo trip: {src} -> {dst}")

    route = route_multimodal(con, src, dst, start_mode=start_mode, end_mode=end_mode)
    if route is None:
        raise SystemExit(f"no multimodal route between {src} and {dst}")

    legs = " → ".join(f"{leg['mode']}({leg['time_s']:.0f}s)" for leg in route["legs"])
    print(f"route {src} → {dst}: {legs}")
    print(f"  total {route['time_s']:.0f}s over {route['length_m']:.0f} m, "
          f"{len(route['transfers'])} transfer(s): "
          + ", ".join(f"{t['from_mode']}→{t['to_mode']} ({t['kind']}) @ {t['node_id']}"
                      for t in route["transfers"]))

    layers = _mode_layers(route, Layer)
    if not layers:
        raise SystemExit("route has no drawable geometry")
    title = f"duckOSM multimodal — {db.stem}: " + " · ".join(
        f"{m} {MODE_COLOR[m]}" for m in MODE_COLOR if any(ly.name == f"route_{m}" for ly in layers))
    index = render_web(layers, str(out_dir), basemap=basemap, title=title)
    if write_serve:
        write_serve(str(out_dir))
    print(f"\nwrote {index}")
    print(f"serve it:  python {out_dir}/serve.py   # then open the printed http URL")
    return index


def main(argv=None):
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--db", default="data/db/sodermalm_pbf.duckdb", help="duckOSM .duckdb file")
    ap.add_argument("--src", type=int, default=None, help="start OSM junction node_id")
    ap.add_argument("--dst", type=int, default=None, help="destination OSM junction node_id")
    ap.add_argument("--out-dir", default=None,
                    help="output dir (default: reports/<db-stem>_multimodal_route)")
    ap.add_argument("--transfer-cost", type=float, default=60.0,
                    help="flat transfer penalty in seconds (used only if mm.* must be built)")
    ap.add_argument("--basemap", default="osm", help="osm | carto vector | satellite (mapstyle)")
    ap.add_argument("--start-mode", default="walking")
    ap.add_argument("--end-mode", default="walking")
    ap.add_argument("--mapstyle-src", default=None,
                    help="path to the mapstyle checkout's src/ (default: ../mapstyle/src)")
    a = ap.parse_args(argv)
    render_multimodal_route(
        a.db, src=a.src, dst=a.dst, out_dir=a.out_dir, transfer_cost=a.transfer_cost,
        basemap=a.basemap, start_mode=a.start_mode, end_mode=a.end_mode,
        mapstyle_src=a.mapstyle_src)


if __name__ == "__main__":
    main()
