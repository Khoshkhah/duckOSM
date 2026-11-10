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

## Ask questions on a page: duckosm ask

```bash
duckosm ask monaco.duckdb        # then open http://127.0.0.1:8791/
```

The command opens the map of the database on a page on your computer, with two parts from [roadstyle](https://github.com/Khoshkhah/roadstyle):

- **Ask the data:** write a question in words. The agent writes the SQL, runs it, and answers in a few sentences. It can also colour roads,
  focus on roads, move the map and draw a chart. Ask uses your own key of a free AI service. The key stays in your browser.
- **Query the tables:** the data dock. One SQL line over a wide table under the map. The roads of the result light on the map.

[Questions to ask the AI agent](ask-questions.md) has examples for each group of tasks.

Roads, buildings and the other objects of the base map are clickable. A click selects the item, and its card is beside the table. There is no popup. A click runs no
query, so the shapes of the last result stay on the map. The page does not change the data.

A result that has a geometry column is drawn on the map, all its rows: the column `geom` of `driving.nodes` (points) and of `features.buildings`
(shapes), and `geometry` of `main.building_faces` (lines). The limit of 1000 is for the rows that the table shows. The map shows every row that the query selects. So you can write "Show all
buildings on the map" or "Show all nodes of the driving network on the map". The shapes have colours: a result with the column `face_type` (the walls) is coloured by it, with the colours of the editor (front green, back red, side grey, party
wall purple, court cyan). For other colours, select a column `color` with a CSS colour, for example `CASE highway WHEN 'primary' THEN '#dc2626' ELSE '#6b7280' END AS color`.
A click on a row of the table selects the item: a row with a geometry column (a node, a building, a wall) is drawn on the map in the select look, the map zooms to it,
and the card beside the table has all the values of the row. The ✕ of the card removes the drawing. A row of a road is selected by roadstyle. Its card has a section
**This row** with all the columns of the row, for example `width_m` of `lane_profile`.
A result with the column `edge_id` is not drawn this way: roadstyle lights its roads on their own shape, so select `edge_id` for roads. A new table with no
geometry column clears the shapes. The switch **On / Off** in the data dock shows or hides the table on the map, with its roads and
its shapes. A closed dock shows nothing on the map.

### Every table on the map

A result that has the id columns of a table, and no geometry column, shows its objects on the map and has a card for each row. The card has the facts of the object,
and a section **Row** with all the columns of the clicked row. The page names the kind of the result under the table, for example "Shown as Turn".

| The result has | A row is | On the map |
|---|---|---|
| `from_edge` and `to_edge` (`edge_graph`, `turn_permission`), `from_edge_id` and `to_edge_id` (`turn_restrictions`), `via_edges` (`turn_path_restrictions`) | a turn | the road in, the junction (or the roads between) and the road out, in three colours |
| `node_id`, `via_node` | a node | the junction. The card of a node has a button **Show H3 cell**. |
| `way_id`, `osm_id` | an OSM way, a building, a base-map object | its shape. A way is all its edges. |
| `route_id` | a bus route | all its edges |
| `h3_cell`, `h3_id` | an H3 cell | the hexagon. The card has the resolution, the parent cell, the centre, the area and the number of nodes inside. |
| `new_edge_id` (`edge_id_map`) | a merged edge | the new edge |
| `ib_link_id` and `ob_link_id` (GMNS `movement`) | a movement | the link in, the node, the link out |

A result with the column `edge_id` lights the roads on their own shape. A result with a geometry column is drawn as it is (see above). To make the page use another kind,
select the column under the name of that kind, for example `SELECT osm_id AS way_id`. The nodes keep their H3 cell at resolution 8, so the card of a larger cell counts
the nodes inside, and the card of a smaller cell says that it has no count. A table that the page does not know shows its rows in the table only.

The page makes the table `extra.boundary_h3` with the H3 cells that cover the boundary at each resolution 0 to 11; `whole_boundary` is true for the one cell that contains
the whole boundary, so the smallest such cell is the row with the largest resolution where it is true. The page computes these cells with h3-js, because DuckDB has no H3 here.

**Reset** (the button in the data dock) empties the table and gives the map its own colours. It also puts back what **Ask** changed on the map (layers, base map, theme, 3D view),
and leaves the camera and the changes that you made by hand.

The table `extra.edits` has the changes of the [fixes file](edit-osm-data.md) (`osm_fixes.yaml` beside the database, or `--fixes`), with the
time of each change. You can ask about the changes and about the data in one question, for example "Which roads did I change, and how many lanes did
they have?". The columns are `kind` (`object`, `wall`, `turn`, `new_road`), `type`, `osm_id`, `what`, `changed` and `note`. The page reads the file
again each time that you load it. A change that you have not built yet is in `extra.edits` but not in the other tables.

## Where the tables are

Each mode has its own schema: `driving.edges`, `walking.edges`, `cycling.edges`, and so on. Always
name the schema, or switch to it with `USE driving;`. A tool that shows no tables is usually looking at
the empty default schema `main`. Builds from a PBF also keep the parsed OSM data in `raw`. Private
roads (driveways, gated streets) and, in driving, bus lanes are in `driving.private_edges` and the like, not in `edges`: you can
see them but not route on them ([why](../concepts/networks.md#access-private-and-forbidden-roads)).
Every table and column: [Database schema](../reference/database.md). To see what one database
holds (modes, counts, time zone, build date), run `duckosm info monaco.duckdb`.

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

Coordinates are longitude/latitude (EPSG:4326). DuckDB's distance function on the sphere expects
latitude first, so flip the geometry:

```sql
-- the roads nearest a point (the Casino de Monte-Carlo: lat 43.7397, lon 7.4270)
SELECT name, highway,
       round(ST_Distance_Sphere(ST_FlipCoordinates(ST_Centroid(geometry)), ST_Point(43.7397, 7.4270))) AS metres
FROM driving.edges ORDER BY metres, edge_id LIMIT 3;
```

| name | highway | metres |
|---|---|---:|
| Allées des Boulingrins | service | 19.0 |
| Allées des Boulingrins | residential | 19.0 |
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
