# networkx

Load the network into [networkx](https://networkx.org) for your own analysis, or save it to a file.
Needs `pip install "duckosm[routing]"`.

| Function | Nodes | Edges |
|---|---|---|
| `to_networkx(con)` | edges (`edge_id`), with `name`, `highway`, `length_m`, `maxspeed_kmh`, `cost_s`, `geometry` (WKT) | legal turns; `weight` is the time of the edge you leave (`weight="length"`: its metres) |
| `to_networkx_nodes(con)` | junctions (`x`, `y` = lon, lat) | roads, keyed by `edge_id`, with every column (osmnx layout, but the length is `length_m`, not `length`) |

Both take `mode=` too.

To save either one to a file (the format comes from the extension):

```bash
duckosm export-graph monaco.duckdb                            # -> monaco_driving.graphml (junctions)
duckosm export-graph monaco.duckdb -g edge -o routing.gpickle # the routing graph
```

GraphML opens in any tool but stores lists and geometry as text; gpickle keeps lists and types but
only loads in Python (add `--geometry shapely` for shapely geometries on the junction graph). From Python: `write_graph(con, "monaco.graphml")`.
