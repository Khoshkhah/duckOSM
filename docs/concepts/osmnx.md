# duckOSM and OSMnx

[OSMnx](https://osmnx.readthedocs.io) is a Python library that downloads a street network from OSM. duckOSM makes a
road network too, with more for routing and simulation. The guide [duckOSM for OSMnx users](../guides/osmnx-users.md) gives
the duckOSM call for each OSMnx call.

| | OSMnx | duckOSM |
|---|---|---|
| Input | the Overpass API, for a small area | an `.osm.pbf` file, up to a country |
| Graph | undirected or directed, one id for each run | directed, one `edge_id` for each direction, the same after each build |
| Turn restrictions | no | yes, in the turn graph `edge_graph` |
| OSM fixes | no | yes |
| Service roads (`drive_service`) | without parking aisles and private roads | all; private roads are drawn but not routed |
| Pedestrian streets open to cars | no | yes, as `living_street` |
| Output | a networkx graph | a DuckDB file, and [networkx](../exports/networkx.md), GIS and simulation files |

## The same roads

On Monaco, the length of road of each `highway` class, inside the boundary of the duckOSM build. OSMnx is
`drive_service` with `truncate_by_edge=True`, the closest filter to duckOSM's driving network.

| `highway` | OSMnx | duckOSM | Difference |
|---|--:|--:|--:|
| residential | 19,993 m | 19,810 m | −183 m |
| service | 12,898 m | 12,023 m | −875 m |
| secondary | 10,451 m | 10,554 m | +103 m |
| tertiary | 8,492 m | 8,492 m | 0 |
| primary | 6,486 m | 6,486 m | 0 |
| other classes | 2,226 m | 1,970 m | −256 m |
| **total** | **60,546 m** | **59,335 m** | **−2.0 %** |

- Where the two filters agree, the lengths agree to the metre.
- **service −875 m:** OSMnx keeps roads tagged `motor_vehicle=private` with no `access` tag. duckOSM reads that tag and
  puts them in `private_edges`.
- Do not compare the counts of edges or nodes: the two tools cut and join the ways by different rules.

## How to compare

Measure the length of each `highway` class, in metres (the UTM zone of the data), inside the same boundary, with one
direction of each two-way road (`WHERE NOT is_reverse`).

```python
import osmnx as ox, duckdb, geopandas as gpd

con = duckdb.connect("area.duckdb", read_only=True); con.execute("LOAD spatial")
area = gpd.GeoSeries.from_wkb(con.execute("SELECT ST_AsWKB(geom) FROM main.boundary").df().iloc[:, 0].map(bytes),
                              crs=4326).union_all()
utm = gpd.GeoSeries([area], crs=4326).estimate_utm_crs()
cut = gpd.GeoSeries([area], crs=4326).to_crs(utm).iloc[0]

def metres(gdf):
    gdf = gdf.to_crs(utm)
    gdf["highway"] = gdf["highway"].map(lambda h: h[0] if isinstance(h, list) else h)
    return gdf.clip(cut).assign(m=lambda d: d.length).groupby("highway")["m"].sum()

G = ox.graph_from_bbox(area.bounds, network_type="drive_service", truncate_by_edge=True)
osmnx = metres(ox.convert.graph_to_gdfs(ox.convert.to_undirected(G), nodes=False))
duck = con.execute("SELECT highway, ST_AsWKB(geometry) AS g FROM driving.edges WHERE NOT is_reverse").df()
duck = metres(gpd.GeoDataFrame(duck, geometry=gpd.GeoSeries.from_wkb(duck.pop("g").map(bytes), crs=4326)))
print((duck - osmnx).round())
```
