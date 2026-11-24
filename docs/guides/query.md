# Query the database

A build is one DuckDB file. Open it with DuckDB from Python, the [DuckDB command line](https://duckdb.org/docs/installation/)
or another tool that reads DuckDB.

```python
import duckdb
con = duckdb.connect("monaco.duckdb", read_only=True)
con.execute("LOAD spatial")                  # for the ST_ functions
con.sql("SELECT count(*) FROM driving.edges").show()
```

| Schema | Holds |
|---|---|
| `driving`, `walking`, `cycling` | the network of each mode: `edges`, `nodes`, `edge_graph`, `private_edges`, ... |
| `raw` | the OSM nodes, ways and relations with all their tags (a build from a PBF) |
| `features` | the base-map layers |
| `main` | the area: boundary, time zone, build data |

Always give the schema, or use `USE driving;`. Private roads and, in driving, bus lanes are in `private_edges`, not in
`edges`: the maps show them, but routing does not use them
([why](../concepts/networks.md#access-private-and-forbidden-roads)). All tables and columns:
[Routable network tables](../reference/database.md). A summary of the database: `duckosm info monaco.duckdb`.

## Ask in words

```bash
duckosm ask monaco.duckdb        # open http://127.0.0.1:8791/ (--port to change it)
```

The page shows the map of the database. It does not change the data.

| Part | Does |
|---|---|
| **Ask the data** | Write a question in words. The AI agent writes the SQL, runs it and gives a short answer. It can also color roads, move the map and draw a chart. It uses your own key of a free AI service, and the key stays in your browser. [Questions to ask the AI agent](ask-questions.md) |
| **Query the tables** | one SQL line, and the result as a table under the map |

The map shows a result by its columns: `edge_id` (roads), a geometry column, `node_id`, `way_id` or `osm_id`, `route_id`,
`h3_cell`, `from_edge` and `to_edge` (a turn). A column `color` gives each row its color. To show a result as a different
kind, rename the column, for example `SELECT osm_id AS way_id`. The table shows the first 1000 rows, and the map shows all
rows.

The page adds `extra.edits` (the changes of the [fixes file](edit-osm-data.md), `--fixes`) and `extra.boundary_h3` (the
H3 cells of the boundary, resolutions 0 to 11).

## Examples

```sql
-- the size of each network
SELECT 'driving' AS mode, count(*) AS edges, round(sum(length_m) / 1000, 1) AS km FROM driving.edges
UNION ALL SELECT 'walking', count(*), round(sum(length_m) / 1000, 1) FROM walking.edges;

-- where you can go next from one edge (the graph of legal turns)
SELECT e.edge_id, e.name FROM driving.edge_graph g JOIN driving.edges e ON e.edge_id = g.to_edge
WHERE g.from_edge = 2226047604433257818;

-- junctions with the most turn restrictions
SELECT via_node, count(*) AS n FROM driving.turn_restrictions GROUP BY 1 ORDER BY 2 DESC LIMIT 5;

-- hospitals and pharmacies in the raw OSM data
SELECT osm_id, tags['name'] AS name FROM raw.nodes WHERE tags['amenity'] IN ('hospital', 'pharmacy');
```

## Places

Coordinates are longitude and latitude (EPSG:4326). `ST_Distance_Sphere` uses latitude first, so flip the geometry:

```sql
SELECT name, highway,
       round(ST_Distance_Sphere(ST_FlipCoordinates(ST_Centroid(geometry)), ST_Point(43.7397, 7.4270))) AS meters
FROM driving.edges ORDER BY meters, edge_id LIMIT 3;
```

## Look up one OSM way

```bash
duckosm way monaco.duckdb 4230100
```

This shows the raw tags and nodes of one OSM way, then all edges that the build made from it, in each mode. A two-way
street gives two edges (`#1f` forward, `#1r` reverse), each with the same `edge_id` in each mode. Use it when a street
looks wrong.

| Option | Does |
|---|---|
| `-m driving` | only this mode; repeat it for more modes |
| `--geom` | add the geometry as WKT |
| `--json` | one JSON object |
| `-o way.csv` | write the table to a `.csv`, `.parquet` or `.json` file |

In Python: `duckosm.open("monaco.duckdb").sql(...)` reads any table; the command `duckosm way DB ID --json` gives the
same table as JSON.
