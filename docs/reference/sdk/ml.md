# duckosm.ml

The graphs of a mode, for graph libraries and graph machine learning. pip extra: `duckosm[graph]`.

| Call | Returns | Today |
|---|---|---|
| `to_networkx(net, mode=None, weight="time")` | the turn graph as a `networkx.DiGraph`: a node is an `edge_id`, an arc a legal turn with its weight (`"time"` in seconds, `"length"` in metres) | yes |
| `to_networkx_nodes(net, mode=None)` | the junction graph as a `networkx.MultiDiGraph`: a node is a junction, an edge a road | yes |
| `to_rustworkx(net, mode=None, graph="turns", weight="time")` | the same graphs as a `rustworkx.PyDiGraph`; `g.attrs["index"]` gives the node index of an id | yes |
| `to_pyg(view, graph="nodes" / "turns", features=[...])`, `to_dgl(view, ...)` | PyTorch Geometric and DGL graphs | later |

```python
g = net.driving.to_networkx()                           # = duckosm.ml.to_networkx(net.driving)
```

The top-level `duckosm.to_networkx(con, ...)` takes a DuckDB connection, as before. Page: [networkx](../../exports/networkx.md).
