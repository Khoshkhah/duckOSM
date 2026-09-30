# Comparing a duckOSM network against OSMnx

OSMnx is the reference street-network library for OSM (Boeing 2025, *Geographical Analysis*), so
it is the natural independent check on what duckOSM extracts. It is **not** a drop-in oracle: the
two use different road filters and different graph models, and a naive comparison reports a ~48%
discrepancy that is entirely explained by those two things.

This document records how to line them up, and what the residual differences mean.

## The two things that must be reconciled first

**1. Road filter.** OSMnx's `network_type` presets are narrower than duckOSM's driving network:

| preset | includes |
|--------|----------|
| `drive` | public drivable streets; **excludes service roads entirely** (also alleys, driveways) |
| `drive_service` | adds service roads, but **still excludes parking aisles and emergency access** |
| duckOSM `driving` | every drivable way including alleys, parking aisles, driveways |

`drive_service` is the closest match, and still not equal.

**2. Boundary handling.** `graph_from_bbox` keeps only nodes inside the bbox, so any edge crossing
the boundary is dropped **whole**. duckOSM clipped with `ST_Intersection` keeps the portion inside.
Pass **`truncate_by_edge=True`** or the comparison is meaningless — this alone accounted for
5.8 km of the 16.2 km gap in the run below.

## Worked comparison

`granville_island.duckdb`, bbox `(-123.142, 49.265, -123.128, 49.275)`, undirected, metres of
centreline clipped to the bbox (EPSG:3857).

| highway | `drive` | `drive` + trunc | `drive_service` + trunc | duckOSM | gap |
|---------|--------:|----------------:|------------------------:|--------:|----:|
| residential | 9,898 | 12,510 | 13,059 | 12,450 | −609 |
| service | 0 | 0 | 5,472 | 9,952 | **+4,480** |
| secondary | 2,540 | 3,453 | 3,453 | 3,894 | +441 |
| trunk | 2,128 | 2,730 | 2,730 | 2,730 | **0** |
| trunk_link | 1,209 | 2,221 | 2,221 | 2,199 | −22 |
| tertiary | 1,558 | 1,874 | 1,874 | 1,596 | −278 |
| living_street | 290 | 290 | 290 | 661 | +371 |
| unclassified | 0 | 344 | 344 | 344 | **0** |
| tertiary_link | 157 | 157 | 157 | 157 | **0** |
| **total** | **17,780** | **23,579** | **29,600** | **33,983** | **+12.9%** |

Naive `drive` vs duckOSM is 47.7% apart. Matched properly it is 12.9%, and the remainder is
accounted for:

- **service +4,480 m** — duckOSM's `parking_aisle` (3,398) + `driveway` (1,054) = 4,452.
  `drive_service` excludes precisely those, so the two figures agree to ~30 m.
- **living_street +371 m** — duckOSM reclasses drivable `highway=pedestrian`
  (`motor_vehicle=yes`) to `living_street` by design; OSMnx keeps the original tag.
- **residential / secondary / tertiary ±600 m** — these largely cancel (−609, +441, −278) and are
  classification-boundary and clipping noise at this scale, not a systematic difference.

`trunk`, `unclassified` and `tertiary_link` match **to the metre** — where the filters agree, the
geometry agrees. That is the signal worth watching: an exact match on unambiguous classes means
extraction and clipping are sound.

## Recipe

```python
import osmnx as ox, duckdb, geopandas as gpd, shapely

bbox = (w, s, e, n)
G = ox.convert.to_undirected(
        ox.graph_from_bbox(bbox, network_type="drive_service", truncate_by_edge=True))
gdf = ox.convert.graph_to_gdfs(G, nodes=False).to_crs(3857)
env = gpd.GeoSeries([shapely.box(*bbox)], crs=4326).to_crs(3857).iloc[0]
gdf["m"] = gdf.clip(env).length          # per-class: groupby highway

con.execute("""
  SELECT highway, ST_AsWKB(ST_Intersection(geometry, ST_MakeEnvelope(?,?,?,?))) wkb
  FROM driving.edges
  WHERE NOT is_reverse AND ST_Intersects(geometry, ST_MakeEnvelope(?,?,?,?))""", [*bbox, *bbox])
```

Compare **per highway class**, not totals — a single total hides which filter is responsible.
Use `NOT is_reverse` against an undirected OSMnx graph, or both sides double-count two-way streets.

## What this is and is not good for

**Good for:** confirming geometry and clipping on unambiguous classes; catching a whole class
going missing; sanity-checking a new area build.

**Not good for:** edge or node counts. OSMnx models nodes as OSM junction nodes with ways between
them; duckOSM splits at junctions, contracts degree-2 chains and keys edges by content hash. The
counts are not comparable by construction and no amount of filter matching will make them so.
Compare **length per class**, which is a physical quantity both agree on.

**Cost:** OSMnx is an Overpass API client — ~3 s for a 102-node bbox, network-bound. Fine for a
spot check on a small area, not for anything fleet-wide.
