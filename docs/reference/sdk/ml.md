# duckosm.ml

The graphs of a mode, for graph libraries and graph machine learning. pip extras: `duckosm[graph]` (networkx,
rustworkx), `duckosm[ml]` (torch, torch_geometric).

| Call | Returns | Today |
|---|---|---|
| `to_networkx(net, mode=None, weight="time")` | the turn graph as a `networkx.DiGraph`: a node is an `edge_id`, an arc a legal turn with its weight (`"time"` in seconds, `"length"` in metres) | yes |
| `to_networkx_nodes(net, mode=None)` | the junction graph as a `networkx.MultiDiGraph`: a node is a junction, an edge a road | yes |
| `to_rustworkx(net, mode=None, graph="turns", weight="time")` | the same graphs as a `rustworkx.PyDiGraph`; `g.attrs["index"]` gives the node index of an id | yes |
| `to_pyg(net, mode=None, graph="nodes", features=("length_m", "maxspeed_kmh", "lanes"), node_features=("lon", "lat"))` | a `torch_geometric.data.Data`: `edge_index`, `x`, `edge_attr`, `num_nodes`, `edge_id` (a tensor), `node_id` (`graph="nodes"`), `x_names`, `edge_attr_names` | yes |
| `to_dgl(net, mode=None, graph="nodes", features=...)` | a `dgl.DGLGraph`: `ndata["x"]`, `edata["edge_attr"]`, `edge_id` in `edata` (`"nodes"`) or `ndata` (`"turns"`) | yes (needs dgl) |
| `graph_arrays(net, mode=None, graph="nodes", features=..., node_features=("lon", "lat"))` | the same arrays as numpy, for another library. `node_features` (junction graph): also `in_degree`, `out_degree`, `degree` or a column of the nodes | yes |

| `graph` | A node is | An arc is | `x` | `edge_attr` |
|---|---|---|---|---|
| `"nodes"` | a junction | an edge | `lon`, `lat` | the `features` columns of the edge |
| `"turns"` | an edge | a legal turn of `edge_graph` (the mode's own use) | the `features` columns of the edge | `cost`: the seconds of the first edge |

A text column (`highway`) becomes one 0/1 column for each value (`highway=primary`); a boolean column becomes 0/1;
NULL becomes 0. DuckDB makes the index arrays; Python only makes the tensors.

```python
g = net.driving.to_networkx()                           # = duckosm.ml.to_networkx(net.driving)
data = duckosm.ml.to_pyg(net.driving, graph="turns", features=["length_m", "lanes", "highway"])
```

Notebook: [15. Graph machine learning](https://khoshkhah.github.io/duckOSM/book/15_ml.html). Guide:
[duckOSM for graph machine learning users](../../compare/for-ml-users.md).

duckOSM has no `HeteroData` export. The GMNS db has the tables for one: `node`, `link`, `lane`, `movement` and `signal_controller` ([details](../../compare/for-ml-users.md#heterogeneous-graphs-from-level-3-gmns)).

The top-level `duckosm.to_networkx(con, ...)` takes a DuckDB connection, as before. Page: [networkx](../../exports/networkx.md).
