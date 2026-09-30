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
| OSMnx `drive_service` | yes, but not `service=parking`, `parking_aisle`, `emergency_access` or `private` |
| duckOSM `driving` | all, including parking aisles and driveways; private ones are drawn but not routable |

**Pedestrian streets.** OSMnx's `drive` and `drive_service` leave `highway=pedestrian` out. duckOSM
keeps the pedestrian streets that are open to cars, as `living_street`.

**Access tags.** duckOSM reads the most specific tag (`motorcar`, then `motor_vehicle`, `vehicle`,
`access`). OSMnx drops any way with `motor_vehicle=no`, `motorcar=no` or `access=private`. duckOSM
keeps `access=private` roads in `private_edges`, drawn on its maps but never routed (see
[Access](networks.md#access-private-and-forbidden-roads)).

**Border.** `graph_from_bbox` drops every edge that crosses the box. Pass `truncate_by_edge=True`,
and cut both networks to the same area before measuring. Here the area is duckOSM's own boundary
polygon, because the box around Monaco also holds French roads that duckOSM left out.

**Length.** Measure in a metric CRS, the UTM zone of the data. Web Mercator (EPSG:3857) makes
lengths about 1.5 times too long at this latitude.

## Recipe

Run it in an environment that has `osmnx`, `duckdb` and `geopandas`. It needs a live connection to
the Overpass API.

```python
import osmnx as ox, duckdb, geopandas as gpd, pandas as pd

con = duckdb.connect("area.duckdb", read_only=True)
con.execute("LOAD spatial")

# the area to measure: duckOSM's boundary
area = gpd.GeoSeries.from_wkb(
    con.execute("SELECT ST_AsWKB(geom) FROM main.boundary").df().iloc[:, 0].map(bytes),
    crs=4326).union_all()
utm = gpd.GeoSeries([area], crs=4326).estimate_utm_crs()   # metres (EPSG:32632 for Monaco)
cut = gpd.GeoSeries([area], crs=4326).to_crs(utm).iloc[0]

def metres(gdf, name):
    gdf = gdf.to_crs(utm)
    gdf["highway"] = gdf["highway"].map(lambda h: h[0] if isinstance(h, list) else h)
    gdf["m"] = gdf.clip(cut).length
    return gdf.groupby("highway")["m"].sum().rename(name)

def osmnx_m(network_type, truncate):
    G = ox.graph_from_bbox(area.bounds, network_type=network_type, truncate_by_edge=truncate)
    return metres(ox.convert.graph_to_gdfs(ox.convert.to_undirected(G), nodes=False), network_type)

duck = gpd.GeoDataFrame(con.execute(
    "SELECT highway, ST_AsWKB(geometry) AS geometry FROM driving.edges WHERE NOT is_reverse").df())
duck["geometry"] = gpd.GeoSeries.from_wkb(duck["geometry"].map(bytes), crs=4326)
duck = metres(duck.set_geometry("geometry"), "duckOSM")

t = pd.concat([osmnx_m("drive", False), osmnx_m("drive", True),
               osmnx_m("drive_service", True), duck], axis=1).fillna(0).round()
t.columns = ["drive", "drive + truncate", "drive_service + truncate", "duckOSM"]
t = t[t.sum(axis=1) > 0].sort_values("duckOSM", ascending=False)
t.loc["total"] = t.sum()
print(t.to_string())
```

When a simplified OSMnx edge merges several OSM ways, its `highway` is a list; the recipe takes the
first element. `NOT is_reverse` keeps one direction of each two-way street, to match the undirected
OSMnx graph.

## An example

Measured on Monaco with the recipe above: metres of road inside the boundary of the duckOSM build
(OSM data as of the run; a new run can differ by a few hundred metres).

| `highway` | OSMnx `drive` | `drive` + truncate | `drive_service` + truncate | duckOSM | Difference |
|---|--:|--:|--:|--:|--:|
| residential | 20,108 | 20,132 | 19,993 | 19,810 | −183 |
| service | 0 | 0 | 12,898 | 12,023 | −875 |
| secondary | 10,451 | 10,451 | 10,451 | 10,554 | +103 |
| tertiary | 8,492 | 8,492 | 8,492 | 8,492 | 0 |
| primary | 6,139 | 6,486 | 6,486 | 6,486 | 0 |
| primary_link | 1,018 | 1,018 | 1,018 | 1,018 | 0 |
| unclassified | 427 | 427 | 554 | 427 | −127 |
| secondary_link | 520 | 520 | 520 | 391 | −129 |
| living_street | 89 | 89 | 89 | 89 | 0 |
| tertiary_link | 45 | 45 | 45 | 45 | 0 |
| **total** | **47,289** | **47,660** | **60,546** | **59,335** | **−2.0%** |

The difference is duckOSM minus `drive_service` + truncate.

- **Truncate** adds 371 m, 347 m of it on `primary`: the edges that cross the border.
- **service −875 m**: mostly roads tagged `motor_vehicle=private` without an `access` tag (1,226 m in
  Monaco). duckOSM reads that tag and moves them to `private_edges`; OSMnx drops only
  `access=private`, so it keeps them. Smaller differences go the other way.
- **residential, secondary, unclassified, secondary_link**: differences of 100 to 200 m each.
  Where OSMnx merges several ways into one edge, its `highway` is a list, and the recipe takes the
  first element, so a class can shift to a neighbour from run to run.
- **living_street**: equal here. The extra `living_street` from pedestrian streets open to cars
  (see above) did not show up in Monaco.

`tertiary`, `primary`, `primary_link`, `living_street` and `tertiary_link` match to the metre:
where the filters agree, the geometry agrees.

## What it is good for

Good for checking geometry and clipping, and for noticing a whole road class that went missing.
Not good for counts of edges or nodes: OSMnx also splits and merges, but duckOSM cuts every mode
at the same points, including where a footway or crosswalk meets a road, merges chains by its own
rules, and adds virtual nodes, so its counts can't match OSMnx's. OSMnx downloads from the Overpass
API, so keep the area small.
