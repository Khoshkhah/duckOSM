# duckOSM and OSMnx

[OSMnx](https://osmnx.readthedocs.io) is the standard Python library for street networks from OSM, so
it is a useful independent check of what duckOSM extracts. The two don't use the same road filter or
the same graph model, so compare **length of road per `highway` class**, not totals and not edge
counts.

## Line them up first

**Road filter.** Use OSMnx's `drive_service`, the closest to duckOSM's driving network:

| Network | Service roads |
|---|---|
| OSMnx `drive` | none |
| OSMnx `drive_service` | yes, but not parking aisles or emergency access |
| duckOSM `driving` | all, including parking aisles and driveways |

OSMnx also drops ways tagged `access=private` or `motor_vehicle=no`; duckOSM keeps them (see
[What each network contains](networks.md#which-osm-ways-each-mode-keeps)).

**Border.** `graph_from_bbox` drops every edge that crosses the box. Pass `truncate_by_edge=True`,
and cut both networks to the box before measuring.

## Recipe

```python
import osmnx as ox, duckdb, geopandas as gpd, shapely

bbox = (w, s, e, n)
G = ox.convert.to_undirected(
        ox.graph_from_bbox(bbox, network_type="drive_service", truncate_by_edge=True))
gdf = ox.convert.graph_to_gdfs(G, nodes=False).to_crs(3857)
env = gpd.GeoSeries([shapely.box(*bbox)], crs=4326).to_crs(3857).iloc[0]
gdf["m"] = gdf.clip(env).length                     # then: gdf.groupby("highway")["m"].sum()

con = duckdb.connect("area.duckdb", read_only=True)
con.execute("LOAD spatial")
duck = con.execute("""
  SELECT highway, ST_AsWKB(ST_Intersection(geometry, ST_MakeEnvelope(?,?,?,?))) AS wkb
  FROM driving.edges
  WHERE NOT is_reverse AND ST_Intersects(geometry, ST_MakeEnvelope(?,?,?,?))""", [*bbox, *bbox]).df()
```

`NOT is_reverse` keeps one direction of each two-way street, to match the undirected OSMnx graph.

## An example

Measured on Granville Island, Vancouver: bbox `(-123.142, 49.265, -123.128, 49.275)`, metres of
road inside the box.

| `highway` | OSMnx `drive` | `drive` + truncate | `drive_service` + truncate | duckOSM | Difference |
|---|--:|--:|--:|--:|--:|
| residential | 9,898 | 12,510 | 13,059 | 12,450 | −609 |
| service | 0 | 0 | 5,472 | 9,952 | +4,480 |
| secondary | 2,540 | 3,453 | 3,453 | 3,894 | +441 |
| trunk | 2,128 | 2,730 | 2,730 | 2,730 | 0 |
| trunk_link | 1,209 | 2,221 | 2,221 | 2,199 | −22 |
| tertiary | 1,558 | 1,874 | 1,874 | 1,596 | −278 |
| living_street | 290 | 290 | 290 | 661 | +371 |
| unclassified | 0 | 344 | 344 | 344 | 0 |
| tertiary_link | 157 | 157 | 157 | 157 | 0 |
| **total** | **17,780** | **23,579** | **29,600** | **33,983** | **+12.9%** |

- **service +4,480 m**: about the length of duckOSM's parking aisles (3,398 m) and driveways
  (1,054 m).
- **living_street +371 m**: duckOSM turns `highway=pedestrian` streets open to cars into
  `living_street`; OSMnx keeps the original class.
- **residential, secondary, tertiary**: differences of a few hundred metres that mostly cancel out:
  roads classed differently near the border of the box.

`trunk`, `unclassified` and `tertiary_link` match to the metre: where the filters agree, the
geometry agrees.

## What it is good for

Good for checking geometry and clipping, and for noticing a whole road class that went missing.
Not good for counts of edges or nodes: duckOSM splits roads at junctions, merges chains and adds
virtual nodes, so its counts can't match OSMnx's. OSMnx downloads from the Overpass API, so keep the
area small.
