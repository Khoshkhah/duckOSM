#!/usr/bin/env python
"""Render a clean roadstyle HTML map of a duckOSM network db.

Reads <mode>.edges (geometry + highway class) from any duckOSM .duckdb and renders
publication-quality road cartography (casing + fill by highway class, CARTO basemap,
legend) via the `roadstyle` package.

  python scripts/roadstyle_map.py --db data/db/sodermalm.duckdb
  python scripts/roadstyle_map.py --db data/db/sodermalm.duckdb \
      --mode driving --out reports/sodermalm.html --basemap voyager --theme light
  # colour by a numeric column instead of highway class:
  python scripts/roadstyle_map.py --db net.duckdb --color-by cost_s --cmap viridis

Needs: geopandas + roadstyle (pip install geopandas /home/kaveh/projects/roadstyle).
"""
import argparse
from pathlib import Path

import duckdb
import geopandas as gpd
import roadstyle as rs
from shapely import wkt as shapely_wkt


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--db", required=True, help="duckOSM .duckdb file")
    ap.add_argument("--mode", default="driving", help="schema: driving | walking | cycling")
    ap.add_argument("--out", default=None, help="output .html (default: <db>_<mode>.html)")
    ap.add_argument("--basemap", default="voyager", help="default base map (selected on load)")
    ap.add_argument("--basemaps", default="voyager,positron,esri_gray,osm,satellite",
                    help="comma list of base maps offered as a toggleable layer switcher; "
                         "'' for none. Any of: voyager positron esri_gray osm dark_matter satellite")
    ap.add_argument("--theme", default="light", help="light | dark")
    ap.add_argument("--color-by", default=None, help="numeric/categorical column to colour by")
    ap.add_argument("--cmap", default=None, help="matplotlib cmap for --color-by")
    ap.add_argument("--copy-field", default="edge_id",
                    help="column copied to the clipboard on edge click ('' to disable)")
    ap.add_argument("--title", default=None)
    a = ap.parse_args()

    db = Path(a.db).resolve()
    out = Path(a.out) if a.out else db.with_name(f"{db.stem}_{a.mode}.html")

    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute(f"ATTACH '{db}' AS d (READ_ONLY)")
    cols = [c[0] for c in con.execute(f"SELECT * FROM d.{a.mode}.edges LIMIT 0").description]

    if "highway" not in cols:
        raise SystemExit(f"{a.mode}.edges has no 'highway' column (roadstyle styles by road class)")
    # resolve the id column for click-to-copy + tooltip: explicit --copy-field, else the
    # first of edge_id / id / osm_id / fid that exists (so a renamed id column still works).
    id_col = (a.copy_field if (a.copy_field and a.copy_field in cols)
              else next((c for c in ("edge_id", "id", "osm_id", "fid") if c in cols), None))
    if a.copy_field and a.copy_field not in cols:
        print(f"note: --copy-field '{a.copy_field}' not in {a.mode}.edges"
              + (f"; using '{id_col}'" if id_col else "; copy disabled"))

    # cast the id column to VARCHAR so a 64-bit edge_id > 2^53 survives JS Number precision
    # (else the tooltip/copy round its low digits to a non-existent id).
    sel = (["highway"] + ([f'CAST({id_col} AS VARCHAR) AS "{id_col}"'] if id_col else [])
           + (["COALESCE(name,'') AS name"] if "name" in cols else [])
           + ([a.color_by] if (a.color_by and a.color_by in cols
                               and a.color_by not in (id_col, "highway", "name")) else []))
    df = con.execute(
        f"SELECT {', '.join(sel)}, ST_AsText(geometry) AS wkt FROM d.{a.mode}.edges").df()
    if df.empty:
        raise SystemExit(f"no edges in {a.mode}.edges")
    print(f"{len(df):,} edges in {a.mode}" + (f"  ·  copy field: {id_col}" if id_col else ""))

    df["geometry"] = df["wkt"].map(shapely_wkt.loads)
    g = gpd.GeoDataFrame(df.drop(columns=["wkt"]), geometry="geometry", crs="EPSG:4326")

    tooltip = [c for c in (id_col, "highway", "name", a.color_by) if c and c in g.columns]
    kw = dict(theme=a.theme, basemap=a.basemap, tooltip=tooltip,
              name=a.title or db.stem, legend=True,
              copy_field=id_col)                    # click an edge -> copy this column
    layers = [b.strip() for b in a.basemaps.split(",") if b.strip()]
    if layers:                                        # toggleable base-map layer switcher
        kw["basemaps"] = [a.basemap] + [b for b in layers if b != a.basemap]
    if a.color_by:                                    # data-driven colour overrides class styling
        kw["color_by"] = a.color_by
        if a.cmap:
            kw["cmap"] = a.cmap
    m = rs.render_edges(g, **kw)

    out.parent.mkdir(parents=True, exist_ok=True)
    m.save(str(out))
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
