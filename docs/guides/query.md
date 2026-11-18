# Query the database

A build is one DuckDB file. Open it from Python, the [DuckDB command line](https://duckdb.org/docs/installation/),
or any tool that reads DuckDB (DBeaver, …). The examples on this page use the Monaco network
from [Your first network](../first_network.md). The numbers in the result tables are examples: they change with the date of the OSM file.

```python
import duckdb
con = duckdb.connect("monaco.duckdb", read_only=True)
con.execute("LOAD spatial")                  # for the ST_ functions below
con.sql("SELECT count(*) FROM driving.edges").show()
```

## Ask questions on a page: duckosm ask

```bash
duckosm ask monaco.duckdb        # then open http://127.0.0.1:8791/ (--port to change it)
```

The page shows the map of the database, with two parts from [roadstyle](https://github.com/Khoshkhah/roadstyle):

| Part | Does |
|---|---|
| **Ask the data** | Write a question in words. The AI agent writes the SQL, runs it, and gives a short answer. It can also color roads, move the map and draw a chart. It uses your own key of a free AI service; the key stays in your browser. Examples: [Questions to ask the AI agent](ask-questions.md) |
| **Query the tables** | the data dock: one SQL line, and the result as a table under the map |

The page does not change the data. A click on a road, a building or another object of the base map selects it and
shows its card next to the table.

How the map shows a result:

| The result has | On the map |
|---|---|
| `edge_id` | the roads, on their own shape |
| a geometry column (`geom`, `geometry`) | each row as a point, a shape or a line. A column `face_type` gives each wall its color; a column `color` (a CSS color) gives each row its color |
| `from_edge` and `to_edge`, `from_edge_id` and `to_edge_id`, or `via_edges` | a turn: the road in, the junction, the road out |
| `node_id`, `via_node` | the junction |
| `way_id`, `osm_id` | the OSM way or the base-map object |
| `route_id` | all edges of the bus route |
| `h3_cell`, `h3_id` | the hexagon of the H3 cell |
| `new_edge_id` | the merged edge |
| `ib_link_id` and `ob_link_id` (GMNS `movement`) | the link in, the node, the link out |

The table shows the first 1000 rows. The map shows all rows. To show a result as a different kind, rename the
column, for example `SELECT osm_id AS way_id`. **Reset** in the data dock empties the table and puts back the map
that Ask changed. The switch **On / Off** shows or hides the result on the map.

The page adds two tables:

| Table | Holds |
|---|---|
| `extra.edits` | the changes of the [fixes file](edit-osm-data.md) (`osm_fixes.yaml` next to the database, or `--fixes`): `kind` (`object`, `wall`, `turn`, `new_road`), `type`, `osm_id`, `what`, `changed` (the time), `note`. A change that you did not build yet is only in this table |
| `extra.boundary_h3` | the H3 cells that cover the boundary at resolutions 0 to 11: `resolution`, `h3_cell`, `whole_boundary` (TRUE for the one cell that holds the whole boundary) |

More detail: [the design of the page on GitHub](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/ask_page.md).

## Where the tables are

| Schema | Holds |
|---|---|
| `driving`, `walking`, `cycling` | the network of each mode: `edges`, `nodes`, `edge_graph`, … |
| `raw` | the OSM data of a build from a PBF |
| `features` | the base-map layers |
| `main` | the area: boundary, time zone, build data |

Always name the schema, or select it with `USE driving;`. A tool that opens only the default schema `main` does not
show the networks. Private roads (driveways, gated streets) and, in driving, bus lanes are in `private_edges`, not in
`edges`. The maps show them, but routing does not use them ([why](../concepts/networks.md#access-private-and-forbidden-roads)).

All tables and columns: [Database schema](../reference/database.md). To see the modes, the counts, the time zone and
the build date of a database, run `duckosm info monaco.duckdb`.

## The size of each network

```sql
SELECT 'driving' AS mode, count(*) AS edges, round(sum(length_m) / 1000, 1) AS km FROM driving.edges
UNION ALL SELECT 'walking', count(*), round(sum(length_m) / 1000, 1) FROM walking.edges
UNION ALL SELECT 'cycling', count(*), round(sum(length_m) / 1000, 1) FROM cycling.edges;
```

| mode | edges | km |
|---|---:|---:|
| driving | 2765 | 92.6 |
| walking | 10952 | 244.1 |
| cycling | 10274 | 245.5 |

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
| 8072509973910824634 | Avenue Saint-Romain | residential |
| 3664387098764709418 | Avenue Delphine | residential |

```sql
-- junctions with the most turn restrictions
SELECT via_node, count(*) AS restrictions
FROM driving.turn_restrictions GROUP BY via_node ORDER BY restrictions DESC LIMIT 5;
```

## Places

Coordinates are longitude/latitude (EPSG:4326). DuckDB's distance function on the sphere uses
latitude first, so flip the geometry:

```sql
-- the roads nearest a point (the Casino de Monte-Carlo: lat 43.7397, lon 7.4270)
SELECT name, highway,
       round(ST_Distance_Sphere(ST_FlipCoordinates(ST_Centroid(geometry)), ST_Point(43.7397, 7.4270))) AS meters
FROM driving.edges ORDER BY meters, edge_id LIMIT 3;
```

| name | highway | meters |
|---|---|---:|
| Allées des Boulingrins | service | 19.0 |
| Allées des Boulingrins | residential | 19.0 |
| Avenue de Monte-Carlo | residential | 21.0 |

```sql
-- edges per H3 cell (the cell of each edge's start)
SELECT from_cell, count(*) AS edges FROM driving.edges GROUP BY from_cell ORDER BY edges DESC LIMIT 3;
```

## Raw OpenStreetMap data

A build from a PBF keeps all OSM nodes, ways and relations in `raw`, with all their tags. Use it for objects that are
not roads. An area cut from a larger database has no `raw`.

```sql
SELECT osm_id, tags['name'] AS name, tags['amenity'] AS amenity
FROM raw.nodes WHERE tags['amenity'] IN ('hospital', 'pharmacy') AND tags['name'] IS NOT NULL LIMIT 5;
```

## Look up one OSM way

`duckosm way` shows all the data of a build for one OSM **way** (a street, or a part of a street):
its raw OSM tags and nodes, then all edges that the build made from it, in all modes, as one table.

```bash
duckosm way monaco.duckdb 4230100                   # Avenue Delphine; -m driving for one mode, --geom for WKT, --json for JSON
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

The table above shows some of the columns; the output has all `edges` columns. This two-way street gives two
edges, one for each direction (`#1f` forward, `#1r` reverse). Each edge has the same `edge_id` in the three
modes, with the speed and the travel time of each mode.

```python
from duckosm.query import way_table
way_table(con, 4230100).show()                      # the same table in Python / a notebook
```

Use it when a street looks wrong: where the build cut it, which edges came from it, and the speed and the cost of each
edge. A negative `osm_id` shows a connector edge, which joins an open path end to the network.
