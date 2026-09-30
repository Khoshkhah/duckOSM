# Query the database

A build is one DuckDB file. Open it from Python, the [DuckDB command line](https://duckdb.org/docs/installation/),
or any tool that reads DuckDB (DBeaver, …). Every example on this page was run on the Monaco network
from [Your first network](../first_network.md).

```python
import duckdb
con = duckdb.connect("monaco.duckdb", read_only=True)
con.execute("LOAD spatial")                  # for the ST_ functions below
con.sql("SELECT count(*) FROM driving.edges").show()
```

## Where the tables are

Each mode has its own schema: `driving.edges`, `walking.edges`, `cycling.edges`, and so on. Always
name the schema, or switch to it with `USE driving;`. A tool that shows no tables is usually looking at
the empty default schema `main`. Builds from a PBF also keep the parsed OSM data in `raw`. Private
roads (driveways, gated streets) are in `driving.private_edges` and the like, not in `edges`: you can
see them but not route on them ([why](../concepts/networks.md#access-private-and-forbidden-roads)).
Every table and column: [Database schema](../reference/database.md).

## The size of each network

```sql
SELECT 'driving' AS mode, count(*) AS edges, round(sum(length_m) / 1000, 1) AS km FROM driving.edges
UNION ALL SELECT 'walking', count(*), round(sum(length_m) / 1000, 1) FROM walking.edges
UNION ALL SELECT 'cycling', count(*), round(sum(length_m) / 1000, 1) FROM cycling.edges;
```

| mode | edges | km |
|---|---:|---:|
| driving | 1940 | 92.6 |
| walking | 8948 | 235.3 |
| cycling | 8448 | 243.3 |

## Roads

```sql
-- the fastest roads
SELECT name, highway, maxspeed_kmh, round(length_m) AS length_m
FROM driving.edges WHERE maxspeed_kmh >= 70 ORDER BY maxspeed_kmh DESC, length_m DESC LIMIT 5;

-- the longest edges
SELECT name, highway, round(length_m) AS length_m FROM driving.edges ORDER BY length_m DESC LIMIT 5;

-- one-way and two-way
SELECT oneway, count(*) AS edges FROM driving.edges GROUP BY oneway;
```

## Turns

```sql
-- where you can go next from one edge (the graph of legal turns)
SELECT e.edge_id, e.name, e.highway
FROM driving.edge_graph g JOIN driving.edges e ON e.edge_id = g.to_edge
WHERE g.from_edge = 2226047604433257818;          -- the end of Avenue Delphine
```

| edge_id | name | highway |
|---:|---|---|
| 4105183032679836683 | Avenue Saint-Romain | residential |
| 3664387098764709418 | Avenue Delphine | residential |

```sql
-- junctions with the most turn restrictions
SELECT via_node, count(*) AS restrictions
FROM driving.turn_restrictions GROUP BY via_node ORDER BY restrictions DESC LIMIT 5;
```

## Places

Coordinates are longitude/latitude (EPSG:4326). DuckDB's distance function on the sphere expects
latitude first, so flip the geometry:

```sql
-- the roads nearest a point (the Casino de Monte-Carlo: lat 43.7397, lon 7.4270)
SELECT name, highway,
       round(ST_Distance_Sphere(ST_FlipCoordinates(ST_Centroid(geometry)), ST_Point(43.7397, 7.4270))) AS metres
FROM driving.edges ORDER BY metres LIMIT 3;
```

| name | highway | metres |
|---|---|---:|
| Allées des Boulingrins | service | 19.0 |
| Avenue de Monte-Carlo | residential | 21.0 |
| Avenue de Monte-Carlo | residential | 21.0 |

```sql
-- edges per H3 cell (the cell of each edge's start)
SELECT from_cell, count(*) AS edges FROM driving.edges GROUP BY from_cell ORDER BY edges DESC LIMIT 3;
```

## Raw OpenStreetMap data

Builds from a PBF keep every OSM node, way and relation in `raw`, with all their tags, so you can
reach things that aren't roads (areas cut from a bigger build don't have `raw`):

```sql
SELECT osm_id, tags['name'] AS name, tags['amenity'] AS amenity
FROM raw.nodes WHERE tags['amenity'] IN ('hospital', 'pharmacy') AND tags['name'] IS NOT NULL LIMIT 5;
```

## Look up one OSM way

`duckosm way` shows all the data a build has for one OSM **way** (the OpenStreetMap object for a
street, or a stretch of one): first its raw OSM tags and node list, then every edge built from it,
in every mode, as one table.

```bash
duckosm way monaco.duckdb 4230100                   # Avenue Delphine; -m driving for one mode, --geom for WKT
duckosm way monaco.duckdb 4230100 -o way.csv        # write the table to a file (.csv / .parquet / .json)
```

```text
raw way 4230100: 10 node refs
  tags: {'highway': 'residential', 'name': 'Avenue Delphine'}
  refs: [21923931, 12421715488, 21924090, 25243156, 3625098865, ...]
```

| mode | edge_id | edge_ref | source → target | length_m | maxspeed_kmh | cost_s | walk_type / cycle_type |
|---|---|---|---|---|---|---|---|
| cycling | 2226047604433257818 | 4230100#1f | 21923931 → 21924057 | 88.3 | 15 | 21.2 | mixed_traffic |
| driving | 2226047604433257818 | 4230100#1f | 21923931 → 21924057 | 88.3 | 30 | 10.6 | |
| walking | 2226047604433257818 | 4230100#1f | 21923931 → 21924057 | 88.3 | 5 | 63.6 | shared_road |
| cycling | 3664387098764709418 | 4230100#1r | 21924057 → 21923931 | 88.3 | 15 | 21.2 | mixed_traffic |
| driving | 3664387098764709418 | 4230100#1r | 21924057 → 21923931 | 88.3 | 30 | 10.6 | |
| walking | 3664387098764709418 | 4230100#1r | 21924057 → 21923931 | 88.3 | 5 | 63.6 | shared_road |

(A selection of the columns; the real table has every `edges` column.) This two-way street became two
edges, one per direction (`#1f` forward, `#1r` reverse). Each has the same `edge_id` in all three
modes, with that mode's own speed and travel time.

```python
from duckosm.query import way_table
way_table(con, 4230100).show()                      # the same table in Python / a notebook
```

Use it when a particular street looks wrong: how it was split, which edges came from it, what speed
and cost each got. A negative `osm_id` looks up the short connector edges duckOSM adds to join
dangling paths.
