"""Write docs/notebooks/02_inspect.ipynb: chapter 2 of the notebook manual, "Look inside the file".
The style comes from scripts/notebook_book.py (rules: docs/design/notebook_book.md).

    python scripts/notebook_chapter2.py docs/notebooks/02_inspect.ipynb"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from notebook_book import IMAGES, Chapter  # noqa: E402

ch = Chapter(2, "02_inspect", "Look inside the file",
             "The schemas, the tables, the columns, the edges and SQL of one network file",
             learn="the schemas and tables of a network file; the columns and their descriptions; the numbers of a table; "
                   "the mode views; edges and nodes as GeoDataFrames; SQL on the file; your own DuckDB connection; what `edge_id` is",
             time="about 2 minutes", extras="gis",
             next_=("03_maps", "3. Draw maps", "the live map, the dashboard and the static picture"))

ch.md("""## What is in a network file

A network file is one DuckDB database. A **schema** is a group of tables in it, as a folder holds files. Each mode
that you build is one schema, with the same table names. The other schemas hold the data that all modes share.

| Schema | What it holds | The main tables |
|---|---|---|
| `driving`, `walking`, `cycling` | the network of one mode | `edges` (the roads), `nodes` (the junctions), `edge_graph` (the turns from edge to edge), `ways` and `way_nodes` (the OSM ways of the mode), `private_edges` (roads that the mode cannot use) |
| `main` | the facts of the area | `boundary` (the polygon), `global_junctions` (the nodes of all modes), `visualization_metadata` (the build time, the version, the config) |
| `raw` | the OSM data of the cut | `nodes`, `ways`, `relations` |
| `features` | the base map | `buildings`, `water_polygons`, `land`, `ocean`, `pois` and more |
| `visualization` | the drawing order | `edge_levels` (which road is drawn over which: bridges over tunnels) |
| `bus` | the bus lines | `routes`, `route_edges` |

The `driving` schema has three more tables: `turn_restrictions` (the "no left turn" rules of OSM), `lane_profile`
(the lanes) and `turn_path_restrictions`. A build option that you turn off removes its schema (chapter 1, "The build
options").

The two tables that you use most are `<mode>.edges` and `<mode>.nodes`. An **edge** is one direction of a road
between two nodes. A **node** is a point where edges meet. Each edge has a `source` node and a `target` node, so the
edges and the nodes make a directed graph.

> **Key idea:** a network file is a normal DuckDB database. Everything in this chapter is a read of its tables. You
> can read the same tables with SQL, with pandas or GeoPandas, or with any other DuckDB tool.""")

ch.md("""## Open the network of chapter 1

Chapter 1 builds Monaco with three modes (driving, walking and cycling) and saves it as `monaco.duckdb`. This chapter opens that file with `duckosm.open`: it builds nothing. The chapter compares the three modes.""")
ch.code("""net = duckosm.open("monaco.duckdb")    # the network of chapter 1: Monaco, three modes
net                                                                                               # the card""")
ch.md("""The card names the file, its size (36.5 MB) and the build time. The table has one
row for each mode: driving has 2840 edges, walking 11938 and cycling 11305.""")

ch.md("""## The file and its modes

A `Network` is also the path of its file. `net.path` is a `pathlib.Path`, and each function that takes a file name
takes `net` too. `net.modes` lists the schemas that have a road table. `net.boundary` is the polygon of the area as a
GeoDataFrame.""")
ch.code("""print(net.path.name)       # the name of the file
print(net.modes)           # the modes in the file
net.boundary               # the polygon of the area, with its name, OSM id and area in km2""")
ch.md("""The boundary is one row: the relation 2220322, "Monaco", 2.39 km2, from Nominatim. The
`geometry` column is a MultiPolygon in longitude and latitude (EPSG:4326).""")

ch.md("""## The tables: `net.tables()` and `net.schema()`

`net.tables()` gives the name of each table as `schema.table`. `net.schema()` gives one row for each table, with
the number of rows, the number of columns and a **description**. The build writes each description into the file as
a DuckDB comment. The text is the same as on the reference page
[The database schema](../reference/database.md).""")
ch.code("""tables = net.tables()      # a list of "schema.table"
print(len(tables), "tables")
tables[:8]                 # the first eight""")
ch.code("""import pandas as pd
pd.set_option("display.max_colwidth", 120)    # show more of each description
net.schema()                                  # one row for each table: rows, columns, description""")
ch.md("""**What the output shows.** The file has 53 tables in eight schemas. The largest tables are the turns of walking
(`walking.edge_graph`, 35074 rows) and the OSM nodes (`raw.nodes`, 34461 rows). Some tables have 0 rows: Monaco has
no turn permissions and no virtual nodes, but the table is there, so the same query works on each file.

> **Tip:** the row counts of `net.schema()` are exact: it counts each table. DuckDB's own `duckdb_tables()` gives
> only an estimate.""")

ch.md("""### The tables of one schema

`net.schema("driving")` gives only the tables of one schema. A wrong name gives an error that lists the schemas of
the file.""")
ch.code("""net.schema("main")         # the facts of the area""")
ch.md("""**What the output shows.** `main` has three tables: the boundary (1 row), the junctions of all modes (4326 rows)
and the metadata of the build (1 row). The description of `global_junctions` starts with "The nodes where every
mode cuts its ways". `boundary` has no description: its section of the reference page has no text before its table.""")

ch.md("""## The columns: `net.columns()`

`net.columns(table)` gives the columns of one table: the name, the DuckDB type and the description. Read it before
you write SQL or a filter, to know the names and the types.""")
ch.code("""net.columns("driving.edges")      # the 28 columns of the driving roads""")
ch.md("""**What the output shows.** Each column has its description: for `edge_id` it is the formula
`(hash(osm_id, source, target) >> 1)::BIGINT`, for `source` "the node the edge starts at". The first columns are the keys: `edge_id`, `edge_ref`, `source`, `target` and
`osm_id`. Then come the OSM tags (`highway`, `name`, `oneway`, `lanes`, `surface` ...), the `geometry`, and the
values that the build adds: `length_m`, `maxspeed_kmh` and `cost_s` (the travel time in seconds). The last six
columns are keys of H3 cells and map tiles, for fast spatial reads.

| Column | Type | What it is |
|---|---|---|
| `edge_id` | BIGINT | the number of the edge; the same in every build and in every mode (section "What edge_id is") |
| `edge_ref` | VARCHAR | a readable name, `<osm_id>#<part>` and `f` or `r` for the direction |
| `source`, `target` | BIGINT | the node where the edge starts, and the node where it ends |
| `osm_id` | BIGINT | the OSM way of the edge |
| `highway` | VARCHAR | the road class of OSM: `primary`, `residential`, `footway` ... |
| `oneway` | BOOLEAN | `true` when the road has one direction only |
| `length_m` | FLOAT | the length in meters |
| `maxspeed_kmh` | FLOAT | the speed limit in km/h: the OSM tag, or a default for the road class |
| `cost_s` | FLOAT | the travel time in seconds at `maxspeed_kmh` |
| `geometry` | GEOMETRY | the line of the edge, in longitude and latitude |""")

ch.md("""### Compare the modes

The `edges` tables of the modes have almost the same columns. Set operations on the column names show the
differences.""")
ch.code("""cols = {m: set(net.columns(f"{m}.edges").column_name) for m in net.modes}   # the column names of each mode
print("walking only:", cols["walking"] - cols["driving"])
print("cycling only:", cols["cycling"] - cols["driving"])""")
ch.md("""Walking adds `walk_type` (for example a sidewalk or a crossing). Cycling adds
`cycle_type` and `dismount` (`true` where you must push the bicycle). Every other column is the same in the three
modes, so one query works on each mode.""")

ch.md("""## The numbers of a table: `net.summarize()`

`net.summarize(table)` runs DuckDB's `SUMMARIZE`. It gives one row for each column: the minimum, the maximum, the
approximate number of distinct values, the average, the quartiles, the count and the percentage of NULL values. The
geometry columns are left out. Use it to find the range of a column and the columns that are often empty.""")
ch.code("""s = net.summarize("driving.edges")                                         # one row for each column
s[["column_name", "min", "max", "approx_unique", "null_percentage"]]       # five of its columns""")
ch.md("""**What the output shows.** The driving edges are from 0.77 m to 1078 m long. The speed limits are from 10 to
70 km/h, with 7 different values. `name` is NULL for 32% of the edges (most of them are service roads), `bridge` is
NULL for 99%: only 1% of the driving edges are on a bridge. `approx_unique` is an estimate: `edge_id` shows 2548,
but each of the 2840 edges has its own `edge_id`.""")

ch.md("""## The view of one mode

`net.summary()` gives one row for each mode (chapter 1). `net.driving`, `net.walking` and `net.cycling` (or
`net["driving"]`) are **mode views**: a `ModeView` has the calls of the network, without `mode=`.

| Calls of a mode view | Same as |
|---|---|
| `net.walking.edges()`, `.nodes()` | `net.edges("walking")`, `net.nodes("walking")` |
| `net.walking.map()`, `.plot()`, `.dashboard()` | `net.map("walking")` ... (chapter 3) |
| `net.walking.route(a, b)` and the other routes | `net.route(a, b, mode="walking")` (chapter 7) |
| `net.walking.to_sumo(...)` and the other exports | `net.to_sumo(..., mode="walking")` (chapter 6) |
| `net.walking.summary()` | the row of walking in `net.summary()` |""")
ch.code("""net.summary()          # one row for each mode""")
ch.code("""walk = net["walking"]     # the same as net.walking
walk.summary()            # the row of one mode""")
ch.md("""Walking has 11938 edges and 4589 nodes, 260.6 km in all. Driving and cycling have
35.5 km of one-way roads. Walking has 0.0 km: a person on foot can walk in both directions on each road.

> **Warning:** a call that needs one network (a route, a graph, an export) stops with a `DuckOSMError` when the file
> has more modes and you give none. The message lists the modes and shows `net['driving'].route(...)`. Use a mode
> view, or `mode=`. `edges()`, `nodes()`, `map()` and `plot()` work on all modes at once.""")

ch.md("""## Edges and nodes as tables

`net.edges()` and `net.nodes()` read a table into a **GeoDataFrame**: a pandas DataFrame with a geometry column.
`edge_id` comes as `int64`, the float columns as `float64`, and the rows are in the order of `edge_id` (of `node_id`
for the nodes). You can then use all of pandas and GeoPandas.

| Argument | Default | What it does |
|---|---|---|
| `mode` | `None` | one mode; `None` gives the one mode of the file, or all modes with a `mode` column |
| `where` | `None` | a SQL filter on the columns, for example `"highway = 'primary'"` (only `edges`) |
| `crs` | EPSG:4326 | the coordinate system of the result: an EPSG number, or `"utm"` for the UTM zone of the area |""")
ch.code("""edges = net.driving.edges()                     # all driving edges
print(edges.shape, edges.crs)                   # rows and columns, the coordinate system
edges[["edge_id", "edge_ref", "osm_id", "highway", "name", "length_m", "maxspeed_kmh", "cost_s"]].head()""")
ch.md("""2840 rows and 28 columns, in EPSG:4326 (longitude and latitude). The first rows are
the smallest `edge_id` values.""")

ch.md("""### Filter with `where`

`where` puts a SQL filter into the read. DuckDB reads only the rows that you ask for, so this is faster than a filter
in pandas on a large file.""")
ch.code("""primary = net.edges("driving", where="highway IN ('primary', 'primary_link')")   # the primary roads
print(len(primary), "edges,", round(primary.length_m.sum() / 1000, 1), "km")""")
ch.md("""218 edges, 12.3 km: the `primary` and `primary_link` rows of the SQL table in the
section "SQL on the file" below.

> **Warning:** the text of `where` goes into the query as it is. Give only text that you write yourself, never text
> that a user of your program types.""")

ch.md("""### Meters: `crs="utm"`

The geometry in longitude and latitude has degrees, not meters. `crs="utm"` gives the edges in the UTM zone of the
area (EPSG:32632 for Monaco), so `.length` and `.area` are in meters.""")
ch.code("""utm = net.edges("driving", crs="utm")                        # the same edges, in meters
print(utm.crs)                                               # the UTM zone of Monaco
print(round(utm.length.sum() / 1000, 1), "km from the geometry")
print(round(utm.length_m.sum() / 1000, 1), "km from length_m")""")
ch.md("""The geometry gives 96.3 km and `length_m` gives 96.2 km. The small difference comes
from the UTM projection: `length_m` is measured on the sphere.""")

ch.md("""### All modes in one table

With more modes in the file and no `mode`, `net.edges()` gives the edges of all modes in one GeoDataFrame, with a
`mode` column. A road that more modes can use is in it once for each mode, with the same `edge_id`.""")
ch.code("""all_edges = net.edges()                  # the three modes, with a "mode" column
all_edges["mode"].value_counts()""")
ch.md("""26083 rows: 11938 walking, 11305 cycling and 2840 driving. The table has the
columns of all modes. `walk_type` is empty for driving and cycling.""")

ch.md("""### The nodes

`net.nodes(mode)` gives the junctions of a mode: `node_id`, the point `geometry`, and the keys `h3_cell` and `tile`.
The node ids are OSM node ids. A negative id is a node that the build added (for example to split a road that comes
back to its start).""")
ch.code("""nodes = net.nodes("walking")    # the junctions of the walking network
print(len(nodes), "nodes")
nodes.head(3)""")

ch.md("""## SQL on the file: `net.sql()`

`net.sql(query)` runs a SQL query on the file and gives a pandas DataFrame. The connection is read-only, so a query
cannot change the file. The DuckDB extension `spatial` is loaded, so the `ST_` functions work. Use SQL for counts,
sums and joins: DuckDB computes them in the file, without a read of all the rows into Python.""")
ch.code("""net.sql(\"\"\"
    SELECT highway, count(*) AS edges, round(sum(length_m) / 1000, 1) AS km
    FROM driving.edges
    GROUP BY highway
    ORDER BY edges DESC
\"\"\")                                                     # the road classes of driving""")
ch.md("""Residential streets (919 edges, 31.7 km) and service roads (855 edges, 23.7 km) are
more than half of the driving network. The primary roads have 197 edges and 11.2 km.""")

ch.md("""### Values in a query: `params`

Put a value into a query with `$name` (or `?`) and `params`, not with Python text. DuckDB then reads the value as a
value, and a quote in a name cannot break the query.""")
ch.code("""net.sql("SELECT edge_id, edge_ref, length_m FROM driving.edges WHERE name = $name ORDER BY edge_ref",
        {"name": "Boulevard Albert 1er"})                 # the edges of one street""")

ch.md("""### Spatial SQL

The spatial functions take the geometry columns. Here the area of the boundary in km2, measured in the UTM zone, and
the number of driving edges within 200 m of the casino square.""")
ch.code("""net.sql(\"\"\"
    SELECT round(ST_Area(ST_Transform(geom, 'EPSG:4326', 'EPSG:32632', always_xy := true)) / 1e6, 2) AS km2
    FROM main.boundary
\"\"\")                                                     # the area of Monaco""")
ch.code("""net.sql(\"\"\"
    SELECT count(*) AS edges
    FROM driving.edges
    WHERE ST_DWithin(ST_Transform(geometry, 'EPSG:4326', 'EPSG:32632', always_xy := true),
                     ST_Transform(ST_Point(7.4280, 43.7393), 'EPSG:4326', 'EPSG:32632', always_xy := true), 200)
\"\"\")                                                     # the edges near the casino square""")
ch.md("""The boundary has 2.39 km2, the same value as the `area_km2` column of
`net.boundary`. 223 driving edges are within 200 m of the casino square.

> **Tip:** `always_xy := true` tells DuckDB that the points are longitude first. Without it, `ST_Transform` reads
> EPSG:4326 as latitude first.""")

ch.md("""## Your own DuckDB connection: `net.connect()`

`net.sql()` opens a connection for each query and gives pandas. `net.connect()` gives a read-only DuckDB connection
that stays open, with `spatial` loaded. Use it for many queries in a row, for DuckDB relations, for Arrow
(`.arrow()`) or for Polars (`.pl()`). Close it after use: a `with` block closes it for you.""")
ch.code("""with net.connect() as con:                                          # closed at the end of the block
    rel = con.sql("SELECT edge_id, highway, length_m FROM walking.edges")    # a DuckDB relation: not read yet
    print(rel.count("*").fetchone()[0], "walking edges")                     # DuckDB counts in the file
    print(rel.filter("highway = 'steps'").count("*").fetchone()[0], "of them are steps")
    table = rel.limit(5).to_arrow_table()                                    # an Arrow table, no pandas
print(type(table).__name__, table.num_rows, "rows")""")
ch.md("""11938 walking edges, and 1038 of them are steps. The Arrow table has 5 rows. The
relation reads nothing until you ask for a result. The two counts run in
DuckDB, and only the numbers come to Python. `to_arrow_table()` gives a pyarrow `Table` for tools that read Arrow.

> **Warning:** in DuckDB 1.5, `.arrow()` gives a `RecordBatchReader`, which reads the rows only when you use it.
> Read it to the end (`.read_all()`) before the connection closes. A reader that you do not read keeps the query open,
> and the next query on the file can wait for it. `to_arrow_table()` reads all rows at once.

> **Tip:** you can also open the file with `duckdb.connect(str(net.path), read_only=True)`. Then load the extension
> yourself: `con.load_extension("spatial")`.""")

ch.md("""## What `edge_id` is

`edge_id` is the key of an edge. It is a number of type `BIGINT` (64 bits). The build computes it from three values
of the edge: the OSM way (`osm_id`) and the two nodes (`source`, `target`).

<img src="IMAGES/edge-id.svg" alt="A two-way road is two edges: each direction has its own edge_ref and edge_id" width="100%">

| Property | Why it is important |
|---|---|
| It comes from the OSM data, not from the row number | a new build of the same OSM data gives the same `edge_id` to each edge |
| Each direction has its own number | `source` and `target` change places, so the two edges of a two-way road get different numbers |
| It is the same in each mode | a street with a sidewalk has the same `edge_id` in `driving.edges` and in `walking.edges` |
| It is a `BIGINT` | keep it as `int64` in pandas; a `float64` loses the last digits (chapter 4) |

`edge_ref` is a second name of an edge that people can read. It can change when a way gets more nodes in OSM, so join
your data on `edge_id`, not on `edge_ref`.

> **Key idea:** `edge_id` is the key between duckOSM and your data. It stays the same when you build the area again
> and it is the same in each mode, so data that you join on it stays correct.""".replace("IMAGES", IMAGES))

ch.md("""### Two edges for a two-way road

The OSM way 4230114 is a two-way street. Each part of the way between two nodes gives two edges: `f` in the drawing
direction of the way, `r` against it.""")
ch.code("""net.sql(\"\"\"
    SELECT edge_id, edge_ref, source, target, is_reverse
    FROM driving.edges WHERE osm_id = 4230114 ORDER BY edge_ref
\"\"\")                                                     # the edges of one way""")
ch.md("""Three parts of the way give six edges. The two edges of each part have the same
nodes in the other order, and two very different numbers: `4230114#3f` is 4950030941241418106, `4230114#3r` is
414242052250797. `is_reverse` is `true` for the `r` edges.""")

ch.md("""### Compute an `edge_id`

`duckosm.edge_id_hash(osm_id, source, target)` computes the `edge_id` in Python. The same formula is in each file as
the SQL macro `edge_id_hash`. `duckosm.edge_id_expr()` gives the SQL text of the formula, for your own queries.""")
ch.code("""row = edges.iloc[0]                                                    # one driving edge
print(row.edge_id, duckosm.edge_id_hash(row.osm_id, row.source, row.target))   # the same number
print(duckosm.edge_id_expr())                                          # the formula as SQL""")
ch.code("""net.sql("SELECT count(*) AS different FROM driving.edges WHERE edge_id <> edge_id_hash(osm_id, source, target)")""")
ch.md("""The Python function gives the same number as the column. The SQL macro agrees for
all 2840 driving edges: 0 are different.

| Call | Gives |
|---|---|
| `duckosm.edge_id_hash(osm_id, source, target)` | the `edge_id` of one edge |
| `duckosm.edge_id_expr("osm_id", "source", "target")` | the SQL text of the formula, with your column names |
| `duckosm.edge_id_hash_v1(...)`, `duckosm.edge_id_expr_v1(...)` | the previous formula, with `is_reverse`: to connect old ids to new ids |
| `duckosm.create_edge_id_macro(con)` | writes the two SQL macros into a DuckDB database of yours |

> **Warning:** the formula uses DuckDB's `hash()`. Another DuckDB major version can give other numbers. duckOSM pins
> DuckDB 1.x for this reason. Outside DuckDB, join on the three columns `osm_id`, `source` and `target`.""")

ch.md("""### The same edge in each mode

A road that more modes can use has the same `edge_id` in each mode. A join of two `edges` tables on `edge_id` gives
the roads that the two modes share.""")
ch.code("""net.sql(\"\"\"
    SELECT 'driving and walking' AS modes, count(*) AS edges FROM driving.edges JOIN walking.edges USING (edge_id)
    UNION ALL
    SELECT 'driving and cycling', count(*) FROM driving.edges JOIN cycling.edges USING (edge_id)
    UNION ALL
    SELECT 'all modes, each road once', count(DISTINCT edge_id)
    FROM (SELECT edge_id FROM driving.edges UNION ALL SELECT edge_id FROM walking.edges
          UNION ALL SELECT edge_id FROM cycling.edges)
\"\"\")                                                     # the roads that the modes share""")
ch.md("""2201 of the 2840 driving edges are also walking edges, and 2829 are also cycling
edges. The three modes have 13302 different edges in all, less than the 26083 rows of the three tables, because most
roads are in two or three modes.""")

ch.exercises([
    ("Find the five longest walking edges that have a name. Use `net.sql`.",
     'net.sql("SELECT name, highway, round(length_m) AS m FROM walking.edges WHERE name IS NOT NULL ORDER BY length_m DESC LIMIT 5")'),
    ("Which columns of `walking.edges` are NULL for more than 90% of the rows?",
     's = net.summarize("walking.edges")\ns[s.null_percentage > 90].column_name.tolist()'),
    ("Get the steps of the walking network as a GeoDataFrame in meters, and give their total length in km.",
     'steps = net.walking.edges(where="highway = \'steps\'", crs="utm")\nround(steps.length.sum() / 1000, 1)'),
    ("Count the driving edges that are not in the walking network, by road class.",
     'net.sql("""SELECT highway, count(*) AS n FROM driving.edges\n           WHERE edge_id NOT IN (SELECT edge_id FROM walking.edges)\n           GROUP BY highway ORDER BY n DESC""")'),
])

ch.glossary([
    ("schema", "a group of tables in a DuckDB file: one for each mode, and `main`, `raw`, `features` ..."),
    ("table", "rows and columns in a schema, named `schema.table`, for example `driving.edges`"),
    ("description", "the text of a table or a column, written into the file as a DuckDB comment"),
    ("`ModeView`", "the view of one mode: `net.walking`, with the calls of the network and no `mode=`"),
    ("GeoDataFrame", "a pandas DataFrame with a geometry column (GeoPandas)"),
    ("CRS", "the coordinate system of a geometry: EPSG:4326 (degrees) or a UTM zone (meters)"),
    ("`edge_ref`", "the readable name of an edge, `<osm_id>#<part>f` or `r`; it can change"),
    ("`edge_id`", "the key of an edge: a hash of `osm_id`, `source` and `target`; the same in each build and mode"),
    ("relation (DuckDB)", "a query that DuckDB runs only when you ask for its result"),
])

ch.summary([
    ("`net.path`, `net.modes`, `net.boundary`", "the file, the modes, the polygon of the area"),
    ("`net.tables()`", "the tables, as `schema.table`"),
    ("`net.schema(schema=None)`", "one row for each table: rows, columns, description"),
    ("`net.columns(table)`", "the columns of a table: name, type, description"),
    ("`net.summarize(table)`", "DuckDB's `SUMMARIZE`: the range, the distinct values and the NULLs of each column"),
    ("`net.summary()`, `net.walking`, `net[\"walking\"]`", "one row for each mode; the view of one mode"),
    ("`net.edges(mode, where=, crs=)`", "the edges as a GeoDataFrame"),
    ("`net.nodes(mode, crs=)`", "the nodes as a GeoDataFrame"),
    ("`net.sql(query, params=None)`", "a SQL query on the file, as a DataFrame"),
    ("`net.connect()`", "a read-only DuckDB connection with `spatial`"),
    ("`duckosm.edge_id_hash(...)`, `duckosm.edge_id_expr()`", "the formula of `edge_id`, in Python and as SQL"),
], reference="[Python SDK, What is in the file](../reference/python.md#what-is-in-the-file) and "
             "[The database schema](../reference/database.md)")

ch.write(sys.argv[1])
