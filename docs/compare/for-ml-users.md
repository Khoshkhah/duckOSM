# duckOSM for graph machine learning users

This page is for users of PyTorch Geometric, DGL and city2graph who make road graphs from OSM with their own code. The
module is `duckosm.ml` ([reference](../reference/sdk/ml.md)). The notebook
[15. Graph machine learning](https://khoshkhah.github.io/duckOSM/book/15_ml.html) trains a small graph neural network on Monaco.

```bash
pip install "duckosm[ml]"      # torch and torch_geometric
```

## Traditional approach vs duckOSM

| Task | Traditional approach | duckOSM |
|---|---|---|
| Road graph for PyTorch Geometric | OSMnx → `torch_geometric.utils.from_networkx(G)` | `duckosm.ml.to_pyg(net.driving)` (Level 2 homogenous graph) |
| Align node & edge indices | Manual index remapping (`0..N-1`) | `data.node_id`, `data.edge_id`: indices map directly back to network |
| Categorical feature encoding | Manual one-hot encoding (`pd.get_dummies`) | `features=["highway"]` auto-creates 0/1 columns (`data.edge_attr_names`) |
| Edge-level tasks (line graph) | `nx.line_graph` (ignores turn restrictions) | `graph="turns"`: legal turns only, respecting turn bans & signals |
| Junction features & elevation | Manual degree & DEM calculation | `node_features=["lon", "lat", "in_degree", "out_degree", "degree", "ele"]` |
| Road graph for DGL | `dgl.from_networkx(G)` | `duckosm.ml.to_dgl(net.driving)` |
| Multi-relational urban graphs | Custom ETL stitching links, lanes, signals | No `HeteroData` export; the GMNS db has the `node`, `link`, `lane`, `movement` tables to read with SQL |
| Custom frameworks / NumPy | Manual tensor/graph array extraction | `duckosm.ml.graph_arrays(net.driving)`: NumPy arrays |
| Multimodal transport graphs | Separate queries with disconnected nodes | `net.walking`, `net.cycling`, or `gmns_all`: shared multimodal crossing nodes |
| Visualize predictions on map | Manual GeoPandas merge and plotting | `net.driving.map(color_by=predicted_series)` keyed directly by `edge_id` |

| `graph` | A node is | An arc is | For |
|---|---|---|---|
| `"nodes"` | a junction (`lon`, `lat`) | an edge, with the `features` | tasks on junctions, or on edges as arcs |
| `"turns"` | an edge, with the `features` | a legal turn, with `cost` | tasks on edges: speed, flow, travel time |

A prediction for a node of the turn graph is a value for an edge. Key it by `data.edge_id` and draw it:

```python
data = duckosm.ml.to_pyg(net.driving, graph="turns", features=["length_m", "lanes", "highway"])
predicted = model(data.x, data.edge_index)
net.driving.map(color_by=pd.Series(predicted.detach().numpy(), index=data.edge_id.numpy()))
```

The `edge_id` is the same on each rebuild. So labels joined by `edge_id` (traffic counts, speeds) stay correct after an
OSM update.

## Heterogeneous graphs from Level 3 GMNS

`duckosm.ml.to_pyg()` makes a homogeneous `Data` from the Level 2 network. duckOSM has no `HeteroData` export.

The GMNS db (`<area>_gmns.duckdb`) has the entities of a heterogeneous graph. You can read them with SQL and make the
`HeteroData` yourself:

| GMNS table | One row is | Joins on |
|---|---|---|
| `node` | a junction | `node_id` |
| `link` | one direction of a road | `link_id` (= `edge_id`), `from_node_id`, `to_node_id` |
| `lane` | a lane of a link | `lane_id`, `link_id` |
| `movement` | a legal turn, with its lanes in and out | `mvmt_id`, `node_id`, `ib_link_id`, `ob_link_id` |
| `signal_controller` | a node with traffic lights (OSM has no signal timing) | `controller_id`, `node_id` |

The columns of each table are in the [GMNS tables reference](../exports/gmns_tables.md).

## duckOSM and city2graph

[city2graph](https://github.com/c2g-dev/city2graph) makes graphs for graph neural networks from GeoDataFrames:
streets, buildings, transit, and graphs with more than one type of node. `net.nodes()` and `net.edges()` are
GeoDataFrames, so they go into city2graph with no change. city2graph takes the node id from the index of the nodes,
and the two ends of an edge from a two-level index of the edges:

```python
import city2graph as c2g

nodes = net.driving.nodes().set_index("node_id")
edges = net.driving.edges().set_index(["source", "target"])
data = c2g.gdf_to_pyg(nodes, edges, edge_feature_cols=["length_m", "maxspeed_kmh"], directed=True, multigraph=True)
# Monaco: Data(x=[1769, 0], edge_index=[2, 2844], edge_attr=[2844, 2], pos=[1769, 2], ...)
```

| Use | For |
|---|---|
| `duckosm.ml.to_pyg` | the road network alone: fast, the turn graph, `edge_id` in the tensors |
| city2graph with `net.nodes()`, `net.edges()` | the roads with other city data (buildings, POIs from `duckosm.gis.features`, transit) in one graph |

## DGL

DGL on PyPI has wheels for torch 2.0 to 2.1 only (Linux). For a newer torch, install DGL from its own index
(<https://www.dgl.ai/pages/start.html>). Then `duckosm.ml.to_dgl` gives a `dgl.DGLGraph` with the same arrays.
