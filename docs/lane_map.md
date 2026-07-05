# Lane-level map + route overlay → `to_lane_geojson` + `lane-map` (design)

**Status:** design spec (2026-07-04), **pending sign-off before implementation** (doc-first). Step 2 of
the *"lane-level map + lane-level routing"* goal: **see** the lanes as a road surface and **the
[lane route](lane_routing.md) drawn on them**. Built in the deck.gl / GeoJSON idiom mapstyle already
uses (`GeoJsonLayer` styled by a per-feature `fc` fill color, plus a `ROUTE` overlay), so the same data
drops straight into mapstyle's viewer.

## Two pieces: reusable data + a viewer

### 1. `to_lane_geojson(gmns_db, out_path, mode="driving")` — the lane surface (duckOSM)

Turn `gmns_<mode>.lane` into **lane-surface polygons**: `ST_Buffer` each drive-side offset centerline by
½·width → a ribbon per lane, as a GeoJSON `FeatureCollection`. Per feature, properties mapstyle expects:

| property | value |
|---|---|
| `fc` | RGBA fill by `allowed_uses` — auto `[143,162,180]`, bus `[232,148,74]`, bike `[90,176,230]` |
| `use`, `lane`, `edge_id`, `width` | `allowed_uses`, `lane_num`, `link_id`, `width` (for hover/inspect) |

This is a **standalone data layer**: a deck.gl `GeoJsonLayer` (fill + a thin dark casing = lane
markings) renders it, in *our* viewer **or** mapstyle's (it matches the `fc` convention). CLI:
`duckosm lane-geojson <gmns_db> -o lanes.geojson`.

### 2. `lane-map` — a deck.gl viewer (lanes + route overlay)

A self-contained HTML (deck.gl via CDN `<script>`, the same way mapstyle loads it) that draws:
- **lane polygons** — a `GeoJsonLayer`, fill `fc` + casing (the "geometry sandwich" mapstyle uses for
  roads, one level down);
- **the lane route** (optional) — from [`route_lanes`](lane_routing.md): a `route-halo` + `route`
  `GeoJsonLayer` (bright, on top), with the maneuver list in a side panel;
- pan/zoom, hover a lane for its id/use/width.

```bash
duckosm lane-map sodermalm_pbf_gmns.duckdb -o lanes.html                       # lanes only
duckosm lane-map sodermalm_pbf_gmns.duckdb --route <fromLane>,<toLane> -o lanes.html   # + the routed lanes
```

`to_lane_map(gmns_db, out_html, mode="driving", route=None)` — writes the HTML (inlining the lane +
route GeoJSON so it opens as one file). `route=(from, to)` calls `route_lanes` and overlays it.

## Why deck.gl + GeoJSON (not our canvas viewers)

We already have canvas lane viewers (`gmns-viz`, `gmns-map`), but they're standalone and not the
mapstyle stack. Doing Step 2 as **`fc`-tagged GeoJSON + a `GeoJsonLayer`** means:
- the exact **look and data convention** as mapstyle's road layer (casing+fill, `fc`), so it reads as a
  natural lane-level companion to the road-level map;
- the lane GeoJSON + route GeoJSON **drop directly into mapstyle's `render_merge` viewer** later (as a
  feature layer + the existing `ROUTE`) — zero rework;
- deck.gl handles the polygon count (3,193 lanes) and metric-width styling cleanly.

## Scope

**v1:** `to_lane_geojson` (lane polygons, `fc` by use) + `to_lane_map` (deck.gl viewer with optional
`route_lanes` overlay + maneuver panel). A lane-level map you can open, pan/zoom, and see a routed lane
path on — **completing both halves of the goal**.

**Out of scope → later:** full integration inside mapstyle's `render_merge` (this ships the reusable
GeoJSON that makes it a drop-in); turn-connector geometry on the map (we have the Béziers); animated
route playback.

## Fidelity

Lane **polygons** are geometrically real (offset centerline buffered by width) but **width is the 3.25 m
default** where OSM lacks `width:lanes`, so ribbons are uniform. The route overlay is exactly what
`route_lanes` returns (its permissive-turn caveat carries over). A base road map can sit under the lane
layer, but v1 draws lanes on a plain ground for clarity.

## Tests

`to_lane_geojson` → a `FeatureCollection` with a polygon per lane, each carrying `fc`/`use`/`edge_id`;
bus lanes get the bus color. `to_lane_map` → an HTML file embedding the lane GeoJSON and (with `route=`)
a route FeatureCollection + a non-empty maneuver list; well-formed, opens without external data files.
