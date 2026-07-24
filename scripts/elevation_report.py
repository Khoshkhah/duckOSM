#!/usr/bin/env python
"""Render a roadstyle **elevation report** of an elevation-enriched duckOSM db.

Colours the roads by ground elevation (mean of the edge's `z_from`/`z_to`) on the roadstyle **web**
backend, with a *Colour by* dropdown (Elevation / Class / Max speed / Lanes), a base-map switcher and
hover read-out. If roadstyle's report sidebar is found (from the roadstyle source checkout) it's
injected too, adding the gradient **legend**, road-class filter, search and a selection panel.

The db must first be enriched with elevation:

    duckosm elevation data/db/sodermalm.duckdb          # adds nodes.ele + edges.z_from/z_to
    python scripts/elevation_report.py --db data/db/sodermalm.duckdb

The report title records the DEM source from `main.elevation_metadata`, so a Copernicus vs EU-DTM
build is self-labelling. Needs: geopandas + roadstyle (`pip install geopandas roadstyle`).
"""
import argparse
from pathlib import Path

import duckdb
import geopandas as gpd
import roadstyle as rs
from shapely import wkt as shapely_wkt


def _find_sidebar():
    """roadstyle's ui/report/sidebar.html from the source checkout (not shipped in the pip wheel),
    located relative to the installed roadstyle package. Returns its text, or None if unavailable."""
    try:
        root = Path(rs.__file__).resolve().parents[2]        # …/roadstyle/src/roadstyle → repo root
        p = root / "ui" / "report" / "sidebar.html"
        return p.read_text() if p.exists() else None
    except Exception:
        return None


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--db", required=True, help="elevation-enriched duckOSM .duckdb")
    ap.add_argument("--mode", default="driving", help="schema: driving | walking | cycling")
    ap.add_argument("--out", default=None, help="output .html (default: <db>_<mode>_elevation.html)")
    ap.add_argument("--basemap", default="voyager", help="default base map")
    ap.add_argument("--cmap", default="viridis", help="matplotlib cmap for the elevation ramp")
    ap.add_argument("--sidebar", default=None,
                    help="report sidebar .html to inject (default: auto-locate roadstyle's; "
                         "omit for a plain color-options map)")
    a = ap.parse_args()

    db = Path(a.db).resolve()
    out = Path(a.out) if a.out else db.with_name(f"{db.stem}_{a.mode}_elevation.html")

    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute(f"ATTACH '{db}' AS d (READ_ONLY)")

    cols = [c[0] for c in con.execute(f"SELECT * FROM d.{a.mode}.edges LIMIT 0").description]
    if "z_from" not in cols or "z_to" not in cols:
        raise SystemExit(f"{a.mode}.edges has no z_from/z_to — run `duckosm elevation {db}` first")

    # DEM source label from provenance, so the title says Copernicus / EU-DTM / the --dem file
    try:
        source = con.execute("SELECT source FROM d.main.elevation_metadata").fetchone()[0]
    except Exception:
        source = "DEM"

    # only offer colour-by options for columns that actually exist
    opt_cols = [("Max speed", "maxspeed_kmh", "plasma"), ("Lanes", "lanes", "magma")]
    extra = [c for _, c, _ in opt_cols if c in cols]
    grade = [c for c in ("bridge", "tunnel", "layer") if c in cols]
    sel = (["highway", 'CAST(edge_id AS VARCHAR) AS edge_id',
            "COALESCE(name,'') AS name", "round((z_from + z_to) / 2.0, 2) AS ele"]
           + grade + extra)
    df = con.execute(
        f"SELECT {', '.join(sel)}, ST_AsText(geometry) AS wkt "
        f"FROM d.{a.mode}.edges WHERE z_from IS NOT NULL").df()
    if df.empty:
        raise SystemExit(f"no elevation-carrying edges in {a.mode}.edges")
    print(f"{len(df):,} edges — ele {df.ele.min():.2f}..{df.ele.max():.2f} m ({source})")

    df["geometry"] = df["wkt"].map(shapely_wkt.loads)
    g = gpd.GeoDataFrame(df.drop(columns=["wkt"]), geometry="geometry", crs="EPSG:4326")

    color_options = {"Elevation": {"color_by": "ele", "cmap": a.cmap}, "Class": {}}
    if "maxspeed_kmh" in cols:
        color_options["Max speed"] = {"color_by": "maxspeed_kmh", "cmap": "plasma"}
    if "lanes" in cols:
        color_options["Lanes"] = {"color_by": "lanes", "cmap": "magma"}

    m = rs.render_edges(
        g, backend="web", basemap=a.basemap,
        basemaps=["voyager", "positron", "dark_matter", "osm", "satellite", "blank"],
        basemap_switcher=True, filter_control=False, road_popup=False,
        color_options=color_options, color_active="Elevation",
        tooltip=["name", "highway", "ele"],
        name=f"{db.stem} — road elevation (m, {source})")

    html = m.html
    sidebar = Path(a.sidebar).read_text() if a.sidebar else _find_sidebar()
    if sidebar:
        html = html.replace("</body>", sidebar + "</body>", 1)
    else:
        print("note: report sidebar not found — writing a plain color-options map (no legend panel). "
              "Pass --sidebar <roadstyle>/ui/report/sidebar.html for the full report.")

    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(html)
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
