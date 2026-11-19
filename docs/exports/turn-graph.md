# The turn graph

The turn graph `edge_graph` holds the legal turns of a network: one row for each pair of edges that you can take one
after the other. Edge-based routing runs on it, and the SUMO, GMNS and networkx exports read it.

| Reference and format | |
|---|---|
| Format | the table `<mode>.edge_graph` in the [DuckDB file](https://duckdb.org/docs/stable/internals/storage) of the build; the native build also writes it as [Parquet](https://parquet.apache.org/docs/file-format/) |
| Reference tool | none. The turn rules come from the OSM [restriction relations](https://wiki.openstreetmap.org/wiki/Relation:restriction) |
| Check | each row of `edge_graph` and of the turn tables equals the golden file of Monaco. [How each output is checked](../concepts/checks.md) |

```sql
SELECT from_edge, to_edge, cost, uses FROM driving.edge_graph LIMIT 5;
```

## The tables

Each mode schema (`driving`, `walking`, `cycling`) has its own turn graph. Driving also has the turn restrictions.

| Table | One row is |
|---|---|
| `edge_graph` | one legal turn: `from_edge`, `to_edge`, `cost` (the `cost_s` of `from_edge`), `uses` |
| `turn_restrictions` | driving: one OSM turn restriction, mapped to edges |
| `turn_path_restrictions` | driving: one turn restriction with a via way, as a path of edges |
| `turn_permission` | one turn that `edge_graph` cannot show: a restriction with an exception, a vehicle class or a condition |

The columns: [Database schema](../reference/database.md#edge_graph).

The build removes the turns that a restriction forbids for all vehicles at all times. A U-turn onto the reverse twin
stays in the table.

## The `uses` column

| `uses` | The turn is between |
|---|---|
| `car`, `walk`, `bike` | two edges of the mode (`edges`) |
| `bus`, `private`, `pedestrian`, `delivery` | an edge of the mode and a restricted edge (`private_edges`) with this `access` label |

Routing reads only the rows of the mode. To also read the restricted turns, give `uses`
([networkx](networkx.md#turns-of-the-restricted-edges)).

## Get it

| As | Do |
|---|---|
| a DuckDB table | open the `.duckdb` file of the build |
| a Parquet file | the native build with the format `geoparquet` writes `edge_graph_<mode>.parquet` in the output folder |
| a GraphML file | `duckosm export-graph monaco.duckdb -g edge` ([networkx](networkx.md)) |
| a networkx graph | `to_networkx(con)` ([networkx](networkx.md)) |
| a route | `duckosm route`, `route()`, `Router` ([Route](../guides/route.md)) |

```sql
SELECT count(*) FROM driving.edge_graph WHERE uses = 'car';                          -- the turns that routing reads
SELECT count(*) FROM driving.edge_graph WHERE uses IN ('car', 'bus', 'private');     -- and the restricted turns
```
