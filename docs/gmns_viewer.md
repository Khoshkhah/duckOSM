# GMNS interactive viewer — `duckosm gmns-viz`

Write a **single self-contained HTML page** that reads an exported GMNS `.duckdb` and lets you
explore it in a browser — no server, no tiles, no build step. It makes the GMNS output legible: the
two geometric levels (individual **lanes**, offset by use, vs the **mesoscopic** section +
turn-connector network) are separate **toggleable layers**, and **hovering any line** names it (type,
id, attributes).

```bash
duckosm gmns-viz sodermalm_pbf_gmns.duckdb                 # -> sodermalm_pbf_gmns_viewer.html
duckosm gmns-viz sodermalm_pbf_gmns.duckdb -m cycling -o out.html
```

```python
from duckosm.gmns_viewer import write_viewer
write_viewer("sodermalm_pbf_gmns.duckdb", "reports/viewer.html", mode="driving")
```

Input is a GMNS DuckDB built by [`duckosm gmns`](gmns_export.md) (the meso layer needs
[`--meso`](gmns_meso.md)).

## What it shows

**Layer toggle** (top-left):

- **Lanes** — every lane from the `lane` table drawn on its per-lane offset geometry, coloured by
  `allowed_uses`: **auto** (grey-blue), **bus** (orange), **bike** (blue). This is where you see
  bus/bike lanes and multi-lane roads.
- **Meso** — the mesoscopic network: **section** links (grey, one per macro link, centerline,
  trimmed at junctions), **turn connectors** (cyan, one per legal movement), and **meso nodes**
  (yellow). Shown only when the db has a `meso_<mode>` schema.

The two are kept as **separate layers on purpose** — lanes are *offset* from the centerline while
meso sections/connectors are *on* the centerline, so stacking them reads as connectors "floating"
over the lanes. Toggle between them instead.

**Hover tooltips** — hover any line; it highlights (white) and a tooltip names it:

| Layer | Tooltip shows |
|-------|---------------|
| lane | `Lane · auto/bus/bike`, `link_id`, lane number, turn (from `turn:lanes`) |
| section | `Meso section`, its `M…` id, `macro_link_id`, lane count, facility type |
| connector | `Turn connector`, movement code (T/L/R/U), lane count, inbound lane range, its `X…-…` id |

**Interaction** — scroll to zoom, drag to pan; opens zoomed to a lane-rich (bus/multi-lane)
junction. Theme-aware chrome (light/dark); the map surface is a committed dark "plotter" canvas.

## How it works

`src/duckosm/gmns_viewer.py` reads the `gmns_<mode>.lane` and (if present) `meso_<mode>.meso_link` /
`meso_node` tables, quantizes the geometry to a compact grid, and inlines it as JSON into an HTML
page that draws everything on a `<canvas>` client-side (hit-testing for the hover). It needs only
`duckdb` + the spatial extension — no geopandas, no external assets — so the output opens anywhere
and is fully offline. See [gmns_export.md](gmns_export.md) / [gmns_meso.md](gmns_meso.md) for the
data it reads.
