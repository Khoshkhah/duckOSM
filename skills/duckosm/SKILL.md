---
name: duckosm
description: Build and use routable road networks from OpenStreetMap with duckOSM - one DuckDB file per area holding driving, walking and cycling networks with stable BIGINT edge_ids, the graph of legal turns, turn restrictions and base-map layers - and export them to SUMO, MATSim, GMNS, GeoPackage or networkx. Use when building a road network for an area from an .osm.pbf, querying or routing on a duckOSM database (edges, nodes, edge_graph, private_edges), joining your own data onto edge_ids, or exporting a network to a simulator or GIS.
---

# duckOSM

Turns an OpenStreetMap `.osm.pbf` into one DuckDB file per area: a routable network per mode, with
ids that survive every rebuild. Docs: https://khoshkhah.github.io/duckOSM/ (all pages as one text
file: `/llms-full.txt`; every table and column: `/reference/database/`; every command:
`/reference/cli/`).

## Install

```bash
pip install "duckosm @ git+https://github.com/Khoshkhah/duckOSM"   # not on PyPI yet
pip install "duckosm[routing,viz] @ git+https://github.com/Khoshkhah/duckOSM"   # + route(), maps
```

Extras: `routing` (networkx: `route()`, `Router`, `export-graph`), `viz` (roadstyle maps), `sumo`
(netconvert), `elevation` (rasterio). Optional system tools: `osmium` (cuts a PBF to a boundary;
without it the whole PBF is read) and GDAL `ogr2ogr`. `duckdb` must stay `<2`.

## Build an area

```bash
# a PBF from https://download.geofabrik.de, cut to the area's border
duckosm boundary Monaco --pbf monaco-latest.osm.pbf                 # -> monaco.geojson (by name or --osm-id)
duckosm build --pbf monaco-latest.osm.pbf -b monaco.geojson -m driving -m walking -m cycling
duckosm info monaco.duckdb                                          # what it holds (--json for one JSON object)
```

Monaco takes seconds; a country takes minutes to hours. For many areas of one region, build the
region once and cut areas from it: `duckosm build --source-db region.duckdb -b area.geojson`
(same `edge_id`s). A YAML config (`duckosm init-config my.yaml`, then `duckosm build -c my.yaml`)
holds every option.

## The database

- **One schema per mode**: `driving`, `walking`, `cycling`. Always name it (`driving.edges`); the
  default schema `main` has no network. `duckosm info DB --json` lists what exists.
- `<mode>.edges`: one row per **directed** edge. Key columns: `edge_id` BIGINT, `source`, `target`
  (OSM node ids; negative = a virtual node), `osm_id` (the way), `highway`, `name`, `oneway`,
  `lanes` (per direction), `maxspeed_kmh`, `length_m`, `cost_s` (seconds), `access`, `is_reverse`,
  `refs` (node ids, source to target), `geometry` (LineString, lon/lat, EPSG:4326). A two-way road
  is two edges, one per direction.
- `<mode>.nodes(node_id, geom)`, `<mode>.edge_graph(from_edge, to_edge, via_edge, cost)`: the legal turns
  (banned turns already removed); `driving.turn_restrictions`.
- `<mode>.private_edges`: private roads (driveways, gated streets) and, in driving, bus lanes and
  bus-only roads (`access = 'bus'`). The columns of `edges` without
  the H3 ones; drawn
  on maps, never routed. They are not in `edges` or `edge_graph`.
- `raw.nodes` / `raw.ways` / `raw.relations`: every OSM element with all its `tags` (a MAP), for
  anything that isn't a road.
- `features.*`: base-map layers (buildings, water, land, pois, …). `mm.*`: across modes, after
  `duckosm multimodal`.

## Recipes

```python
import duckdb
con = duckdb.connect("monaco.duckdb", read_only=True)
con.execute("LOAD spatial")

# the edge nearest a point (ST_Distance_Sphere wants lat first: flip)
near = """SELECT edge_id FROM driving.edges
          ORDER BY ST_Distance_Sphere(ST_FlipCoordinates(ST_Centroid(geometry)), ST_Point(?, ?)) LIMIT 1"""
a = con.execute(near, [43.7285, 7.4155]).fetchone()[0]          # lat, lon
b = con.execute(near, [43.7480, 7.4400]).fetchone()[0]

from duckosm import route                                       # needs the routing extra
r = route(con, a, b)                                            # over legal turns; mode="walking" too
r["time_s"], r["length_m"], r["edges"]

from duckosm import route_points                                # between two points, exact:
r = route_points(con, (7.4155, 43.7285), (7.4400, 43.7480), radius_m=50)   # (lon, lat)
r["time_s"], r["start"]["access_m"]                             # walk to the road + partial edges

# where you can go next from an edge
con.sql(f"SELECT to_edge FROM driving.edge_graph WHERE from_edge = {a}").show()
```

```bash
duckosm way monaco.duckdb 4230100 --json      # one OSM way: raw tags + its edges in every mode
duckosm sumo monaco.duckdb                    # -> sumo/monaco.net.xml (edge ids = edge_id)
duckosm matsim monaco.duckdb                  # -> monaco_network.xml.gz
duckosm gmns monaco.duckdb                    # -> monaco_gmns.duckdb (lanes, movements)
duckosm export-gis monaco.duckdb              # -> monaco.gpkg
duckosm viz monaco.duckdb -m driving          # -> reports/monaco_driving_network.html
```

Join your own data the stable way: carry `(osm_id, source, target)` and join on them, or keep the
`edge_id` itself (`SELECT ... FROM driving.edges e JOIN my_data d USING (edge_id)`).

## Traps

- **`edge_id` is a 64-bit hash, not a row number.** Keep it BIGINT everywhere: a float column or
  JavaScript rounds it (above 2**53), and a pandas column mixing ids and None becomes float64
  (use `Int64`). It is the same in every mode for the same road piece; across modes the key is
  `(mode, edge_id)`.
- **`cost_s` uses the speed limit**: no traffic, no junction delay.
- **`oneway` is per edge.** A one-way road has no reverse edge; walking ignores `oneway` (only
  `oneway:foot`), so the same road can be two-way in walking.
- **Not every road is in every mode**: a main road without a sidewalk tag isn't walkable; forbidden
  roads leave that mode; private ones are in `private_edges`. Check `raw.ways` tags before
  assuming a bug.
- **Open the database `read_only=True`** when reading, so a build or another reader isn't blocked.
- **Export ids**: SUMO / MATSim / GMNS use `edge_id` as their link id; GeoPackage keeps it as a
  64-bit column, a shapefile only as text.
- An unknown config key warns and a wrong choice value (`modes`, `clip.predicate`, …) stops the
  build: read the message, don't retry blindly.
