#!/usr/bin/env python3
"""Overlay all modes (driving / cycling / walking) of ONE duckOSM db in a single map.

Companion to visualize_all.py (which does one map per AREA): this draws the three mode
networks of a single db together, each in its own colour, so you can eyeball where a road
is drive-only / cycle-only / all-three (the cross-mode edge_id alignment). Reuses
visualize_all's fetch()/view/boundary helpers.

    python scripts/visualize_modes.py <db.duckdb> [--out reports/<name>_allmodes.html]
"""
import argparse, sys
from pathlib import Path
import pydeck as pdk

sys.path.insert(0, str(Path(__file__).resolve().parent))
from visualize_all import fetch, meta_view, boundary_geojson, bbox_of, zoom_for, CARTO_LIGHT

# drawn bottom -> top so the thin top layer stays visible over the wide bottom one.
MODES = [("walking", [56, 132, 255], 6), ("cycling", [29, 209, 161], 4), ("driving", [255, 159, 67], 2)]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("db")
    ap.add_argument("--out", default=None)
    args = ap.parse_args()
    db = Path(args.db)
    out = Path(args.out) if args.out else Path("reports") / f"{db.stem}_allmodes.html"

    layers, present, allbb = [], [], [180, 90, -180, -90]
    for mode, color, width in MODES:
        df = fetch(db, mode)
        if df is None or df.empty:
            print(f"  {mode}: no edges"); continue
        df = df.copy(); df["color"] = [color] * len(df); df["mode"] = mode
        layers.append(pdk.Layer("PathLayer", df, pickable=True, get_path="path",
                                get_color="color", get_width=width, width_min_pixels=1,
                                width_max_pixels=10, auto_highlight=True))
        b = bbox_of(df)
        allbb = [min(allbb[0], b[0]), min(allbb[1], b[1]), max(allbb[2], b[2]), max(allbb[3], b[3])]
        present.append((mode, color, len(df)))
        print(f"  {mode}: {len(df):,} edges")

    view = meta_view(db)
    if view:
        lon, lat, zoom = view
    else:
        lon, lat = (allbb[0] + allbb[2]) / 2, (allbb[1] + allbb[3]) / 2
        zoom = zoom_for(max(allbb[2] - allbb[0], allbb[3] - allbb[1]))
    bnd = boundary_geojson(db)
    if bnd:
        layers.append(pdk.Layer("GeoJsonLayer", bnd, stroked=True, filled=False,
                                get_line_color=[20, 20, 20, 220], line_width_min_pixels=2))
    deck = pdk.Deck(layers=layers,
                    initial_view_state=pdk.ViewState(longitude=lon, latitude=lat, zoom=zoom, pitch=0),
                    map_provider="carto", map_style=CARTO_LIGHT,
                    tooltip={"html": "<b>{mode}</b> — {highway}: {name}<br/>{length_m} m",
                             "style": {"backgroundColor": "#161b22", "color": "#e6edf3", "fontSize": "12px"}})
    out.parent.mkdir(parents=True, exist_ok=True)
    deck.to_html(str(out), open_browser=False)
    rows = "".join(
        f'<div style="display:flex;align-items:center;margin:3px 0"><span style="width:22px;height:4px;'
        f'background:rgb({c[0]},{c[1]},{c[2]});margin-right:8px;border-radius:2px"></span>'
        f'{m} <span style="opacity:.6;margin-left:6px">{n:,}</span></div>' for m, c, n in present)
    legend = (f'<div style="position:fixed;top:12px;left:12px;z-index:9999;background:#161b22ee;'
              f'color:#e6edf3;font:13px/1.35 system-ui,sans-serif;padding:12px 14px;border-radius:8px;'
              f'border:1px solid #30363d;box-shadow:0 2px 8px #0006">'
              f'<b>{db.stem} — all modes</b><div style="margin-top:6px">{rows}</div>'
              f'<div style="opacity:.55;margin-top:6px">driving on top · walking underneath</div></div>')
    out.write_text(out.read_text().replace("<body>", "<body>" + legend, 1))
    print(f"\nWROTE: {out}")


if __name__ == "__main__":
    main()
