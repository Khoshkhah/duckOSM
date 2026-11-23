# duckOSM for OSMnx users

This guide provides the direct duckOSM equivalent for every OSMnx 2 function. Operations are located in the core package (`duckosm`), the CLI (`duckosm`), and the analysis module (`duckosm.analysis` — [reference](../reference/sdk/analysis.md)). The notebook [9. Street network analysis](https://khoshkhah.github.io/duckOSM/book/09_analysis.html) demonstrates these workflows on Monaco.

```bash
pip install "duckosm[analysis]"      # networkx, matplotlib, geopandas
```

```python
import duckosm
from duckosm import analysis as an

# Build or open the network (simultaneously extracts driving, walking, and cycling in one pass)
net = duckosm.network("Monaco")
drive = net.driving

# Street network statistics
stats = an.stats(drive)

# Convert to native OSMnx MultiDiGraph if needed for specialized NetworkX algorithms
G = an.to_osmnx(drive)
```

## Get a Network

| OSMnx 2 call | duckOSM CLI / Python SDK | What duckOSM does |
|---|---|---|
| `ox.graph_from_place("Monaco", network_type="drive")` | `net = duckosm.network("Monaco"); drive = net.driving` | Builds the 3 modes (`driving`, `walking`, `cycling`) in one build |
| `ox.graph_from_bbox(bbox)` | `duckosm.network(bbox=(west, south, east, north))` | Spatial bounding box query |
| `ox.graph_from_point((lat, lon), dist=500)` | `duckosm.network(point=(lon, lat), dist_m=500)` | Circular area around coordinate |
| `ox.graph_from_xml("file.osm")` | `duckosm.network(pbf="file.osm.pbf")` | Fast native PBF parsing rather than XML |
| `network_type="walk"`, `"bike"` | `net.walking`, `net.cycling` | Co-located networks sharing identical junction node IDs |
| `ox.load_graphml(path)` | `duckosm.open("data/monaco.duckdb")` | Opens a built DuckDB network file |
| `ox.geocode_to_gdf("Monaco")` | `net.boundary` | Polygon boundary of the queried area |

## Read and Inspect Data

| OSMnx 2 call | duckOSM Python SDK | What duckOSM does |
|---|---|---|
| `ox.graph_to_gdfs(G)` | `net.edges("driving")`, `net.nodes("driving")` | Returns clean GeoDataFrames with stable 64-bit `edge_id` |
| `ox.project_graph(G)` | `net.edges("driving", crs="utm")` | Re-projects to local UTM or custom EPSG |
| `ox.add_edge_speeds(G)`, `ox.add_edge_travel_times(G)` | Columns `maxspeed_kmh` and `cost_s` | Computed during initial build with per-road class speed profiles |
| `ox.distance.add_edge_lengths(G)` | Column `length_m` | Pre-computed ellipsoidal lengths in metres |
| `ox.simplify_graph(G)` | Built-in during build | Non-destructive simplification preserving crossings for multimodal parity |

## Spatial and Network Analysis

| OSMnx 2 call | duckOSM `analysis` call | Description |
|---|---|---|
| `ox.basic_stats(G, area=a)` | `an.stats(drive, area_m2=a)` | Dual-view statistics table (`osmnx` view and `duckosm` view) |
| `ox.stats.count_streets_per_node(G)` | `an.streets_per_node(drive)` | Node degree distributions |
| `ox.bearing.add_edge_bearings(G)` | `an.bearings(drive)` | Compass bearings for all directed road edges |
| `ox.bearing.orientation_entropy(Gu)` | `an.orientation_entropy(drive)` | Street network grid entropy score |
| `ox.plot_orientation(Gu)` | `an.plot_orientation(drive)` | Polar rose chart of street orientations |
| `ox.distance.nearest_nodes(G, X, Y)` | `an.nearest_nodes(drive, X, Y)` | Vectorized nearest node search with distance in metres |
| `ox.distance.nearest_edges(G, X, Y)` | `an.nearest_edges(drive, X, Y)` | Returns stable `edge_id` for each query point |
| `nx.strongly_connected_components(G)` | `an.components(drive)` | Identifies disconnected subgraphs |
| `ox.truncate.largest_component(G)` | `an.largest_component(drive)` | Creates a new DuckDB network pruned to the main component |
| `ox.truncate.truncate_graph_bbox(G, bbox)` | `an.clip(net, bbox=(west, south, east, north))` | Spatially clips network to bounding box |
| `ox.truncate.truncate_graph_polygon(G, poly)` | `an.clip(net, polygon=poly)` | Clips network to arbitrary polygon boundary |
| `ox.truncate.truncate_graph_dist(G, node, 500)` | `an.clip(net, point=(lon, lat), dist_m=500)` | Radial network clipping |
| `ox.elevation.add_node_elevations_raster(G, tif)` | `duckosm elevation <db> --dem <tif>`<br/>`an.add_elevation(net, dem="dem.tif")` | Samples raster DEM, adding node elevation (`ele_m`) and edge grades |
| `ox.elevation.add_edge_grades(G)` | `an.grades(drive)` | Gradient percentages (`grade_pct`) and absolute slopes |
| `ox.utils_geo.sample_points(Gu, 200)` | `an.sample_points(drive, 200)` | Random spatial point sampling along edges |

## Routing and Visualization

| OSMnx 2 call | duckOSM Python SDK | What duckOSM does |
|---|---|---|
| `ox.routing.shortest_path(G, a, b)` | `drive.route(a, b)` | In-process Rust router, with the OSM turn restrictions |
| `ox.routing.route_to_gdf(G, route)` | `route.edges` | The edges of the route as a GeoDataFrame, in route order |
| `ox.plot_graph(G)` | `net.plot("driving")` (static PNG) or `net.map("driving")` (interactive HTML) | Fast MapLibre / Lonboard WebGL maps |
| `ox.plot_graph_route(G, route)` | `net.map("driving", routes=route)` | Overlays route geometry on interactive vector basemap |
| `ox.save_graph_geopackage(G, path)` | `net.to_gis("out/monaco.gpkg")` | Exports GeoPackage with edge and node layers |

## Architectural Differences

| Topic | OSMnx | duckOSM |
|---|---|---|
| **Graph model** | Node-centric `MultiDiGraph` (edges lack dedicated routing IDs) | Directed edge graph with stable 64-bit `edge_id` hashes |
| **Turn restrictions** | Ignored (routes may make illegal U-turns or banned turns) | Enforced via OSM turn relations and custom YAML rules |
| **Multimodal networks** | 3 separate downloads; nodes are not co-located across modes | Single extraction; roads, footways, and cycleways share connection nodes |
| **Data source & limits** | Overpass API (rate limited; practical limit ~metropolitan area) | Local `.osm.pbf` extracts (unlimited size; scales to entire countries) |
| **Storage & memory** | In-memory NetworkX Python objects (high RAM consumption) | Persistent columnar DuckDB database (queryable via SQL without loading into RAM) |
| **ID stability** | `(u, v, key)` changes every download or OSM update | `edge_id` survives rebuilds and OSM updates ([stable edge IDs](../concepts/edge-ids.md)) |
