# networkx

Load the network into [networkx](https://networkx.org) for your own analysis, or save it to a file.
The functions need `pip install "duckosm[networkx]"`. The command `duckosm export-graph` writes GraphML
with no extra.

| Function | Nodes | Edges |
|---|---|---|
| `to_networkx(con)` | edges (`edge_id`), with `name`, `highway`, `length_m`, `maxspeed_kmh`, `cost_s`, `geometry` (WKT) | legal turns; `weight` is the time of the edge you leave (`weight="length"`: its metres) |
| `to_networkx_nodes(con)` | junctions (`x`, `y` = lon, lat) | roads, keyed by `edge_id`, with every column (osmnx layout, but the length is `length_m`, not `length`) |

Both take `mode=` too.

| Library | Function | Use it for |
|---|---|---|
| networkx | `to_networkx`, `to_networkx_nodes` | the most tools and examples (osmnx, momepy) |
| rustworkx | `to_rustworkx(con, graph="turns")`, `to_rustworkx(con, graph="junctions")` | an API like networkx with a Rust engine: faster for large areas. Needs `pip install "duckosm[rustworkx]"` |
| petgraph (Rust) | `duckosm_route::graphs::to_petgraph` | Rust programs ([Rust SDK](../reference/rust.md)) |

The three libraries get the same two graphs. In rustworkx a node has an index: `g.attrs["index"][edge_id]` gives it.

To save either one to a file (the format comes from the extension):

```bash
duckosm export-graph monaco.duckdb                            # -> monaco_driving.graphml (junctions)
duckosm export-graph monaco.duckdb -g edge -o routing.graphml # the routing graph
```

GraphML opens in any tool but stores lists and geometry as text. gpickle keeps lists and types but
only loads in Python: write it from Python, `write_graph(con, "monaco.gpickle")` (add `geometry="shapely"` for shapely
geometries on the junction graph).


## Turns of the restricted edges

The edge graph has only the mode's own turns. To add the turns into and out of the restricted edges
(`private_edges`: `bus`, `private`, `pedestrian`, `delivery`), give `uses`:

```python
G = to_networkx(con, "driving", uses="pedestrian,private")   # CLI: duckosm export-graph --graph edge --uses pedestrian,private
G.nodes[edge_id]["access"]                                    # 'pedestrian' on a restricted edge; absent on a usable edge
```

A restricted edge is a node with its `cost_s` as the weight of its arcs. Do not route cars over them: use this to look at what
meets at a junction. [Database reference](../reference/database.md#edge_graph).
