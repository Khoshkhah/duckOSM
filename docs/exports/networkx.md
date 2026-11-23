# networkx

Load the network into a graph library for your own analysis, or save it as GraphML or gpickle, with your OSM fixes.

| Reference and format | |
|---|---|
| Format | a [networkx](https://networkx.org) or [rustworkx](https://www.rustworkx.org) graph in Python, a [petgraph](https://docs.rs/petgraph) graph in Rust, or a [GraphML](http://graphml.graphdrawing.org/) or gpickle file |
| Check | each graph has the nodes and arcs of the database tables. [How each output is checked](../concepts/checks.md) |

```bash
duckosm export-graph monaco.duckdb                              # -> monaco_driving.graphml, the junction graph
duckosm export-graph monaco.duckdb -g edge -o routing.graphml   # the turn graph
duckosm export-graph monaco.duckdb -f gpickle                   # a networkx pickle: keeps the lists and the types, Python only
```

```python
G = duckosm.to_networkx(con)            # the turn graph; pip install "duckosm[networkx]"
G = duckosm.to_networkx_nodes(con)      # the junction graph
```

## The two graphs

| Graph | A node is | An arc is |
|---|---|---|
| turn graph (`to_networkx`, `-g edge`) | an edge (`edge_id`), with `name`, `highway`, `length_m`, `maxspeed_kmh`, `cost_s`, `geometry` | a legal turn. `weight` is the time of the edge you leave |
| junction graph (`to_networkx_nodes`, the default of `export-graph`) | a junction, with `x`, `y` (longitude, latitude) | a road, keyed by `edge_id`, with every column of `edges`, as in OSMnx |

Routing needs the turn graph, because only it knows the turn restrictions.

## The libraries

| Library | Function | Use it for |
|---|---|---|
| networkx | `to_networkx`, `to_networkx_nodes` | the most tools and examples (OSMnx, momepy) |
| rustworkx | `to_rustworkx(con, graph="turns")` or `graph="junctions"` | the same graphs, faster for large areas. `pip install "duckosm[rustworkx]"` |
| petgraph | `duckosm_route::graphs::to_petgraph` | Rust programs ([Rust SDK](../reference/rust.md)) |

## Speed and memory

Mac mini (Apple M6, 12 cores, 16 GB).

| Area (input DuckDB) | Output | Time | Memory | Output size |
|---|---|---|---|---|
| Kentucky (~1.2 GB) | `export-graph` (GraphML) | 13 s | 2.9 GB | 1.9 GB |
| | `to_networkx()` | 16 s | 5.6 GB | |
| France (~35 GB) | both | not run: the Mac mini has too little free disk | | |

`export-graph` writes the GraphML file while DuckDB reads the rows, so its memory stays near DuckDB's limit of 2 GB. Before
this change, Kentucky needed 9.3 GB. The file has the same bytes.

networkx keeps each node and arc as a Python object, so it is for a city or a region: France needs about 27 GB. For a
country, use rustworkx, petgraph or [`duckosm route`](../guides/route.md).

## Turns of the restricted edges

The turn graph has only the turns of the mode. To add the turns into and out of the restricted edges (`bus`, `private`,
`pedestrian`, `delivery`), give `uses`: `to_networkx(con, uses="pedestrian,private")`, or
`duckosm export-graph -g edge --uses pedestrian,private`. A restricted edge then has the attribute `access`.
