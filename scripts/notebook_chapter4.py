"""Write docs/notebooks/04_join_by_edge_id.ipynb: chapter 4 of the notebook manual, "Join your data by edge_id".
The style comes from scripts/notebook_book.py (rules: docs/design/notebook_book.md).

    python scripts/notebook_chapter4.py docs/notebooks/04_join_by_edge_id.ipynb"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from notebook_book import Chapter  # noqa: E402

ch = Chapter(4, "04_join_by_edge_id", "Join your data by edge_id",
             "Traffic counts, speeds or sensor data on the edges, in pandas, in SQL and on the map",
             learn="why `edge_id` is the key of your data; the type of `edge_id`; a join in pandas and in SQL; the same data "
                   "in more modes; a map and a dashboard of your data; a file of your data; the ids after a rebuild",
             time="about 2 minutes", extras="viz",
             next_=("05_osm_fixes", "5. Fix the OSM data once", "a fixes file that every build and every output takes"))

ch.md("""## The goal of this chapter

You have data about roads from another source, for example a file of traffic counts from the city: one number for
each road and each direction. You want this data on the duckOSM network, so that you can sort the roads by it, query
it with SQL and color the map by it. At the end of the chapter you can:

1. give each row of your data its `edge_id`, the key of a duckOSM edge;
2. join your data to the edges in pandas or in SQL;
3. color the map and the dashboard by your data;
4. build the network again and find your data on the same roads.

## Your data and the edges

Much data about roads has one value for each road and each direction: traffic counts, measured speeds, noise, the
results of a model. To use it with a duckOSM network, each value needs the edge that it belongs to. The key of an
edge is `edge_id` (chapter 2, "What edge_id is").

| Key | Stable after a new build? | Same in each mode? | Use it to join? |
|---|---|---|---|
| `edge_id` | yes: it comes from `osm_id`, `source` and `target` | yes | **yes** |
| `edge_ref` (`4230114#3f`) | no: the part number changes when the way gets more nodes | yes | no, only to read |
| the row number | no: another build or another filter gives another order | no | no |
| `osm_id` | yes | yes | no: one way has many edges, in two directions |
| the geometry | almost: a small change of a node moves it | yes | no: use map matching (chapter 11) |

The flow of this chapter: you make the data once with `edge_id` in it, keep it in a file, and join it to the network
each time that you need it. The join is the same in pandas, in SQL and on the map.

> **Key idea:** keep `edge_id` in your data, and join on it. The same `edge_id` is in each new build of the same OSM
> data and in each mode, so your data stays on the correct roads.

The counts in this chapter are made up. Real counts come from sensors: chapter 11 shows how to put GPS points and
sensor data onto the edges.""")

ch.md("""## Build the network of this chapter

The chapter uses the driving network of Monaco.""")
ch.code("""net = duckosm.network(osm_id=2220322, modes=["driving"], name="monaco")    # get and build the driving network of Monaco
net                                                   # the card""")

ch.md("""## Make the counts

An external file gives one count for each direction of the main roads. Here the counts are made with random numbers
that do not change from run to run: a primary road has approximately 900 vehicles per hour, a secondary road 500 and
a tertiary road 250. The result is a table with two columns: `edge_id` and the count.""")
ch.code("""import numpy as np

main = net.edges(where="highway IN ('primary', 'secondary', 'tertiary')")      # the main roads
base = main.highway.map({"primary": 900, "secondary": 500, "tertiary": 250})   # vehicles per hour, by road class
rng = np.random.default_rng(1)                                                 # made-up numbers that do not change
counts = main[["edge_id"]].assign(vehicles_per_hour=(base * rng.uniform(0.5, 1.5, len(main))).round().astype(int))
counts.head()                                                                  # edge_id (int64) and the count""")
ch.md("""Each row is one edge: one direction of one part of a road. The table has 1013 rows,
one for each primary, secondary and tertiary edge.""")

ch.md("""### The type of `edge_id`

`edge_id` is a 64-bit integer, with up to 19 digits. A `float64` keeps only approximately 16 digits, so a float
changes most ids. The cell turns the ids into floats and back, and counts the ids that changed.""")
ch.code("""print(counts.dtypes)                                            # edge_id is int64
broken = counts.edge_id.astype("float64").astype("int64")       # through a float, and back
print((broken != counts.edge_id).sum(), "of", len(counts), "ids changed")""")
ch.md("""1007 of the 1013 ids changed: a join on them finds almost no edges. Only the 6
small ids stay the same.

> **Warning:** keep `edge_id` as `int64` (`BIGINT` in SQL). A float column, a spreadsheet program or a JSON reader
> that reads each number as a float changes the ids. pandas reads a CSV file of integers as `int64`, and Parquet
> keeps the type. In a spreadsheet, keep the column as text.""")

ch.md("""## Join on `edge_id`

`pandas.merge` joins the counts to the edges. `how="left"` keeps all edges: an edge without a count gets `NaN`. Then
you have all columns of the edge next to its count.""")
ch.code("""gdf = net.edges().merge(counts, on="edge_id", how="left")                    # the edges with their counts
gdf.nlargest(5, "vehicles_per_hour")[["edge_id", "name", "highway", "vehicles_per_hour"]]    # the five largest counts""")
ch.md("""**What the output shows.** The five largest counts (1307 to 1324) are on primary roads, near the top of their
range (900 × 1.5 = 1350): the Boulevard du Larvotto, the tunnels Pasteur and Albert II, and two edges without a name. `gdf` is still a GeoDataFrame: it has the geometry of each edge.""")
ch.code("""print(gdf.vehicles_per_hour.notna().sum(), "of", len(gdf), "edges have a count")
gdf.groupby("highway").vehicles_per_hour.mean().dropna().round()       # the mean count of each road class""")
ch.md("""1013 of the 2840 driving edges have a count. The mean count of each class is near
its base: approximately 900, 500 and 250.""")

ch.md("""### Join in SQL

The same join works in DuckDB. `con.register` gives your DataFrame a table name on the connection, so a query can join
it to the tables of the file. DuckDB computes the join, and only the result comes to Python.""")
ch.code("""with net.connect() as con:                                      # a read-only connection to the file
    con.register("counts", counts)                              # your DataFrame as the table "counts"
    by_class = con.sql(\"\"\"
        SELECT e.highway, count(*) AS edges, round(avg(c.vehicles_per_hour)) AS mean_vph,
               round(sum(c.vehicles_per_hour * e.length_m) / 1000) AS vehicle_km_per_hour
        FROM driving.edges e JOIN counts c USING (edge_id)
        GROUP BY e.highway ORDER BY mean_vph DESC
    \"\"\").df()
by_class""")
ch.md("""**What the output shows.** 197 primary, 429 secondary and 387 tertiary edges, with mean counts of 872, 507
and 253 vehicles per hour. `vehicle_km_per_hour` is the count times the length: the distance that all vehicles drive
on the class in one hour.""")

ch.md("""### Your data in a file

Keep your data in a Parquet file: it keeps `edge_id` as `int64`. `net.sql` can read a Parquet file in the query, so
you do not need to read it into pandas first.""")
ch.code("""import os
os.makedirs("out", exist_ok=True)
counts.to_parquet("out/counts.parquet")                                     # your data, with its types
net.sql(\"\"\"
    SELECT e.name, e.highway, c.vehicles_per_hour
    FROM driving.edges e JOIN 'out/counts.parquet' c USING (edge_id)
    WHERE e.name IS NOT NULL
    ORDER BY c.vehicles_per_hour DESC LIMIT 5
\"\"\")                                                                         # the file, joined in SQL""")
ch.md("""The five largest counts on named roads, from 1287 to 1312 vehicles per hour. DuckDB reads the Parquet file with the
path in the query. The path is from the folder of the notebook.""")

ch.md("""## Color the map by the counts

`color_by` takes your own data keyed by `edge_id` (chapter 3, "Color the roads by a column"). A Series with `edge_id`
as its index is the short form. The edges without a count are gray. Run the cell to see the live map: you can zoom,
and click a road to see its columns. This page shows the static picture of the map.""")
ch.code("""by_edge = counts.set_index("edge_id").vehicles_per_hour    # the counts as a Series keyed by edge_id
net.map(color_by=by_edge, cmap="magma_r")                  # a map, colored by the count""")
ch.md("""The color bar is the count in vehicles per hour. The dark lines are the primary
roads with the largest counts. The thin gray lines are the edges without a count: the residential streets and the
service roads.""")

ch.md("""### The other forms of `color_by`

`color_by` also takes a DataFrame with `edge_id` and one value column, or the name of a column of the edges. Use
`highlight` to show only some edges, for example the edges with more than 1000 vehicles per hour.""")
ch.code("""net.map(color_by=counts, cmap="magma_r")      # a DataFrame: edge_id and one value column""")
ch.code("""busy = counts[counts.vehicles_per_hour > 1000].edge_id      # the edges with more than 1000 vehicles per hour
print(len(busy), "busy edges")                              # the number of busy edges
net.map(highlight=busy)                                     # in orange, the rest gray""")
ch.md("""The DataFrame gives the same map as the Series. 65 edges are busy, all on primary
roads, because only a primary road can have more than 1000 vehicles per hour here.""")

ch.md("""### Save the map

`net.save_map` writes the same map as one HTML file, with the same options (chapter 3, "Save a map as a file"). Send
the file to a person who has no Python.""")
ch.code("""net.save_map("out/counts.html", color_by=by_edge, cmap="magma_r")    # the same map as one HTML file""")

ch.md("""## A dashboard of your data

The dashboard is the map with a panel at the left (chapter 3, "The dashboard"). With `color_by`, the roads have the
colors of your counts, and when you click a road, its card in the panel shows its count in the row `vehicles_per_hour`,
next to the OSM data of the road. Use it to look at single roads: which road has a high count, and what kind of road
it is.""")
ch.code("""net.dashboard(color_by=by_edge, cmap="magma_r")    # the dashboard, colored by the count; click a road for its card""")
ch.md("""On this page the dashboard is a picture, so you cannot click it. Run the cell in your notebook to use the panel.""")

ch.md("""## The same data in more modes

A street with a sidewalk has the same `edge_id` in the driving and in the walking network. So counts that you join to
the driving network are also on the walking network, without a second join key. Here the walking network of Monaco is
built into a second file.""")
ch.code("""walk = duckosm.network(osm_id=2220322, modes=["walking"], name="monaco_walk")   # the walking network
on_walk = counts.edge_id.isin(walk.edges().edge_id)                            # the counts that find a walking edge
print(on_walk.sum(), "of", len(counts), "counted edges are also walking edges")""")
ch.md("""**What the output shows.** 612 of the 1013 counted edges are also in the walking network: the main roads with
a sidewalk. The other 401 edges have no walking twin. 55 of them are in tunnels, where walking is not allowed. On
others, OSM can draw the sidewalk as a separate way: then the walking network uses that way, not the road.""")

ch.md("""## The ids stay after a rebuild

Build Monaco again into a new file. The same counts join to the same edges, with no change.""")
ch.code("""again = duckosm.network(osm_id=2220322, modes=["driving"], name="monaco_again")    # the same area, built again into a new file
counts.edge_id.isin(again.edges().edge_id).all()                              # each count still finds its edge""")
ch.md("""**What the output shows.** `True`: each of the 1013 counts finds its edge in the new file. When the OSM data
changes (a new extract, or a fix of chapter 5), only the edges of the changed ways get new ids. Chapter 5 shows how to
list them with `net.diff`, and chapter 11 shows `duckosm.mobility.edge_id_map`, which connects old ids to new ids.

> **Tip:** to move data from an old build to a new build of other OSM data, join on `edge_id` first. Only the rows
> that find no edge need more work.""")

ch.exercises([
    ("Give each driving edge a made-up speed (for example `maxspeed_kmh` times a random number from 0.6 to 1.0) and "
     "draw the map of the speeds.",
     'edges = net.edges()\nrng = np.random.default_rng(2)\n'
     'speed = edges.set_index("edge_id").maxspeed_kmh * rng.uniform(0.6, 1.0, len(edges))\n'
     'net.map(color_by=speed.rename("speed_kmh"), cmap="RdYlGn")'),
    ("Write the counts to a CSV file, read it again with pandas, and check that the ids are the same.",
     'counts.to_csv("out/counts.csv", index=False)\nimport pandas as pd\n'
     'back = pd.read_csv("out/counts.csv")\nback.edge_id.dtype, back.edge_id.equals(counts.edge_id)'),
    ("Find the sum of the counts on each named road, with SQL on the Parquet file.",
     'net.sql("""SELECT e.name, sum(c.vehicles_per_hour) AS vph\n'
     '           FROM driving.edges e JOIN \'out/counts.parquet\' c USING (edge_id)\n'
     '           WHERE e.name IS NOT NULL GROUP BY e.name ORDER BY vph DESC LIMIT 10""")'),
])

ch.glossary([
    ("join", "putting two tables together on a key column"),
    ("key", "the column that tells which rows belong together: here `edge_id`"),
    ("left join", "a join that keeps all rows of the first table; rows without a match get NULL or `NaN`"),
    ("`int64`, `BIGINT`", "a 64-bit integer: the type of `edge_id` in pandas and in DuckDB"),
    ("Parquet", "a column file format that keeps the types of the columns"),
    ("`register`", "a DuckDB call that gives a DataFrame a table name on a connection"),
    ("vehicles per hour", "the number of vehicles that pass one point of a road in one hour"),
])

ch.summary([
    ("`net.edges(where=...)`", "the edges, filtered in SQL"),
    ("`edges.merge(data, on=\"edge_id\", how=\"left\")`", "your data on the edges, in pandas"),
    ("`con.register(\"name\", df)` in `net.connect()`", "your DataFrame as a table in SQL"),
    ("`net.sql(\"... JOIN 'file.parquet' USING (edge_id)\")`", "your data from a Parquet file, joined in SQL"),
    ("`net.map(color_by=series, cmap=...)`", "a map colored by your data"),
    ("`net.map(highlight=ids)`", "some edges in orange"),
    ("`net.dashboard(color_by=series, cmap=...)`", "the dashboard colored by your data; the road card shows the value"),
    ("`net.save_map(path, color_by=...)`", "the map of your data as one HTML file"),
    ("`counts.edge_id.isin(other.edges().edge_id)`", "check that your data finds its edges in another file"),
], reference="[Python SDK, Maps](../reference/python.md#maps) and [Stable edge ids](../concepts/edge-ids.md)")

ch.write(sys.argv[1])
