#!/usr/bin/env python3
"""Render each duckOSM network as its OWN interactive HTML map, for one-by-one checking.

Auto-discovers every ``data/db/*.duckdb`` EXCEPT a blocklist (default: the
country-scale ``sweden`` / ``estonia`` extracts) and writes a separate
self-contained HTML per network, each centered/zoomed on its own area over a
light street basemap, so the extracted roads can be eyeballed against the real
street grid. Also writes an ``index.html`` to click through them one by one.

Usage:
    python scripts/visualize_all.py                 # all DBs except sweden,estonia
    python scripts/visualize_all.py --exclude sweden estonia sodermalm
    python scripts/visualize_all.py --mode driving  # network/schema to draw
    python scripts/visualize_all.py --combined      # also write one overlaid map
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import duckdb
import pandas as pd
import pydeck as pdk
import pydeck.bindings.json_tools as _jt
from shapely import wkt
from shapely.geometry import MultiLineString

# pydeck pretty-prints the embedded geometry JSON (indent=2), bloating the
# standalone HTML ~5x. Force compact serialization.
_jt.serialize = lambda s: _jt.json.dumps(
    s, sort_keys=True, default=_jt.default_serialize, separators=(",", ":")
)

# Distinct, high-contrast colours [r,g,b], cycled if there are more DBs.
PALETTE = [
    [255, 159, 67], [56, 132, 255], [29, 209, 161], [165, 94, 234],
    [255, 71, 87], [254, 211, 48], [34, 211, 238], [232, 67, 147],
]
CARTO_LIGHT = "https://basemaps.cartocdn.com/gl/positron-gl-style/style.json"


def fetch(db_path: Path, mode: str) -> pd.DataFrame | None:
    con = duckdb.connect(str(db_path), read_only=True)
    con.execute("INSTALL spatial; LOAD spatial;")
    schemas = {r[0] for r in con.execute(
        "SELECT schema_name FROM information_schema.schemata").fetchall()}
    if mode not in schemas:
        con.close()
        return None
    cols = {r[0] for r in con.execute(f"DESCRIBE {mode}.edges").fetchall()}
    name_col = "name" if "name" in cols else "'' AS name"
    hw_col = "highway" if "highway" in cols else "'road' AS highway"
    len_col = ("round(length_m,1) AS length_m" if "length_m" in cols
               else "0.0 AS length_m")
    df = con.execute(f"""
        SELECT ST_AsText(geometry) AS wkt_geom, {name_col}, {hw_col}, {len_col}
        FROM {mode}.edges
    """).df()
    con.close()
    if df.empty:
        return None

    def to_path(w: str):
        try:
            g = wkt.loads(w)
            if isinstance(g, MultiLineString):
                g = max(g.geoms, key=lambda p: p.length)
            return [[round(x, 6), round(y, 6)] for x, y in g.coords]
        except Exception:
            return []

    df["path"] = df["wkt_geom"].map(to_path)
    df = df[df["path"].map(len) >= 2].drop(columns=["wkt_geom"])
    return df


def meta_view(db_path: Path):
    """Hand-tuned view the DB stored in main.visualization_metadata, or None."""
    con = duckdb.connect(str(db_path), read_only=True)
    try:
        r = con.execute("SELECT center_lon, center_lat, initial_zoom "
                        "FROM main.visualization_metadata").fetchone()
    except Exception:
        r = None
    con.close()
    if r and all(v is not None for v in r):
        return float(r[0]), float(r[1]), int(r[2])
    return None


def boundary_geojson(db_path: Path):
    """The admin polygon the extract was clipped to, as a GeoJSON dict (or None)."""
    con = duckdb.connect(str(db_path), read_only=True)
    con.execute("INSTALL spatial; LOAD spatial;")
    gj = None
    for q in ("SELECT boundary_geojson FROM main.visualization_metadata",
              "SELECT ST_AsGeoJSON(geometry) FROM main.boundary"):
        try:
            row = con.execute(q).fetchone()
            if row and row[0]:
                gj = row[0]
                break
        except Exception:
            continue
    con.close()
    if not gj:
        return None
    return {"type": "Feature", "geometry": json.loads(gj), "properties": {}}


def bbox_of(df: pd.DataFrame):
    b = [180.0, 90.0, -180.0, -90.0]  # minx,miny,maxx,maxy
    for p in df["path"]:
        for x, y in p:
            b[0], b[1] = min(b[0], x), min(b[1], y)
            b[2], b[3] = max(b[2], x), max(b[3], y)
    return b


def zoom_for(span: float) -> int:
    for thresh, z in [(0.05, 13), (0.1, 12), (0.2, 11), (0.4, 10), (0.8, 9)]:
        if span < thresh:
            return z
    return 8


def write_map(df, out: Path, title: str, color, subtitle: str, view_xyz=None,
              boundary=None):
    if view_xyz is not None:                       # DB's hand-tuned view
        lon, lat, zoom = view_xyz
    else:                                          # fallback: derive from bbox
        b = bbox_of(df)
        lon, lat = (b[0] + b[2]) / 2, (b[1] + b[3]) / 2
        zoom = zoom_for(max(b[2] - b[0], b[3] - b[1]))
    view = pdk.ViewState(longitude=lon, latitude=lat, zoom=zoom, pitch=0)
    df = df.copy()
    df["color"] = [color] * len(df)
    layers = [pdk.Layer("PathLayer", df, pickable=True, get_path="path",
                        get_color="color", get_width=3, width_min_pixels=1,
                        width_max_pixels=8, auto_highlight=True)]
    if boundary is not None:                        # admin-boundary outline on top
        layers.append(pdk.Layer(
            "GeoJsonLayer", boundary, stroked=True, filled=False,
            get_line_color=[20, 20, 20, 220], line_width_min_pixels=2,
            get_line_width=2))
    deck = pdk.Deck(
        layers=layers, initial_view_state=view,
        map_provider="carto", map_style=CARTO_LIGHT,
        tooltip={"html": "<b>{highway}</b>: {name}<br/>{length_m} m",
                 "style": {"backgroundColor": "#161b22", "color": "#e6edf3",
                           "fontSize": "12px"}})
    out.parent.mkdir(parents=True, exist_ok=True)
    deck.to_html(str(out), open_browser=False)
    overlay = f"""
<div style="position:fixed;top:12px;left:12px;z-index:9999;background:#161b22ee;
  color:#e6edf3;font:13px/1.35 system-ui,sans-serif;padding:12px 14px;
  border-radius:8px;border:1px solid #30363d;box-shadow:0 2px 8px #0006">
  <div style="display:flex;align-items:center;font-weight:700;font-size:15px">
    <span style="width:14px;height:14px;border-radius:50%;
      background:rgb({color[0]},{color[1]},{color[2]});margin-right:8px"></span>{title}</div>
  <div style="opacity:.7;margin-top:4px">{subtitle}</div>
</div>"""
    out.write_text(out.read_text().replace("<body>", "<body>" + overlay, 1))
    return len(df)


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    ap = argparse.ArgumentParser()
    ap.add_argument("--db-dir", default=str(root / "data/db"))
    ap.add_argument("--exclude", nargs="*", default=["sweden", "estonia"])
    ap.add_argument("--mode", default="driving")
    ap.add_argument("--out-dir", default=str(root / "data/output/networks"))
    args = ap.parse_args()

    excl = {e.lower() for e in args.exclude}
    db_files = sorted(p for p in Path(args.db_dir).glob("*.duckdb")
                      if p.stem.lower() not in excl)
    if not db_files:
        raise SystemExit(f"No databases in {args.db_dir} after excluding {excl}")
    out_dir = Path(args.out_dir)
    print(f"{len(db_files)} networks (excluded: {sorted(excl)}) -> {out_dir}\n")

    made = []
    for i, db in enumerate(db_files):
        color = PALETTE[i % len(PALETTE)]
        df = fetch(db, args.mode)
        if df is None or df.empty:
            print(f"  - {db.stem:12s}: no '{args.mode}' edges, skipped")
            continue
        df["db"] = db.stem
        out = out_dir / f"{db.stem}.html"
        view = meta_view(db)
        n = write_map(df, out, f"{db.stem} ({args.mode})", color,
                      f"{len(df):,} edges — black outline = admin boundary",
                      view_xyz=view, boundary=boundary_geojson(db))
        made.append((db.stem, out, n, color))
        src = "metadata" if view else "bbox"
        print(f"  - {db.stem:12s}: {n:7,} edges -> {out.name} "
              f"({out.stat().st_size/1e6:.1f} MB, view={src})")

    # Index page to click through them one by one.
    cards = "".join(
        f'<a href="networks/{out.name}" style="display:block;text-decoration:none;'
        f'color:#e6edf3;background:#161b22;border:1px solid #30363d;border-radius:8px;'
        f'padding:14px 16px;margin:8px 0">'
        f'<span style="width:14px;height:14px;border-radius:50%;display:inline-block;'
        f'background:rgb({c[0]},{c[1]},{c[2]});margin-right:10px;vertical-align:middle"></span>'
        f'<b style="font-size:16px">{name}</b>'
        f'<span style="opacity:.6;margin-left:10px">{n:,} edges</span></a>'
        for name, out, n, c in made)
    index = out_dir.parent / "index.html"
    index.write_text(f"""<!doctype html><meta charset="utf-8">
<title>duckOSM networks</title>
<body style="background:#0e1117;font:14px/1.4 system-ui,sans-serif;max-width:640px;
  margin:40px auto;padding:0 16px">
<h2 style="color:#e6edf3">duckOSM &mdash; {args.mode} networks (check one by one)</h2>
<p style="color:#8b949e">excluded: {', '.join(sorted(excl))}. Click a network to open its map.</p>
{cards}</body>""")

    print(f"\nIndex (click through one by one): {index}")


if __name__ == "__main__":
    main()
