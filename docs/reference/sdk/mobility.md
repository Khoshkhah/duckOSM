# duckosm.mobility

Mobility data on the roads: GPS traces, traffic sensors and your own data per edge. Each function takes a `ModeView`
(`net.driving`), a `Network` with one mode, or a path with `mode=`. The snap, the map matching and the edge id map run
in Rust (`rust/duckosm-core/src/matching.rs`). Install: `pip install "duckosm[analysis]"`.

| Call | Returns | Their tool |
|---|---|---|
| `snap(view, points, radius_m=None)` | The rows of `points` with `edge_id`, `fraction` (0 at the start of the edge, 1 at its end), `distance_m`, `snap_x`, `snap_y`: the nearest edge of each point, one point at a time | own code |
| `match(view, traces, trace="trace_id", time="time", radius_m=50, sigma_m=5, beta_m=10, method="hmm")` | `Match(points, edges)`: `points` are the rows with the columns of `snap`; `edges` is the route of each trace (`trace_id`, `seq`, `edge_id`), with the edges between the points | leuvenmapmatching, mappymatch, gps-dtw |
| `join(view, data, on="edge_id", how="inner")` | A GeoDataFrame of the edges with the columns of `data` (a DataFrame with `edge_id`, or a Series keyed by `edge_id`) | `merge` |
| `aggregate(view, matched, time="hour", value=None)` | `edge_id`, `time` (the start of the bin), `count`, and `<value>_mean` | movingpandas, own code |
| `edge_id_map(old_net, new_net, tol_m=2)` | `old_edge_id`, `new_edge_id`, `share`: the old edges on the edges of a new build | none |

`points` and `traces`: a GeoDataFrame (any CRS), a DataFrame with the columns `x` (lon) and `y` (lat) (other names with
`x=`, `y=`), or a list of `(lon, lat)`.

## The map matching

`match` uses the hidden Markov model of Newson and Krumm (2009, "Hidden Markov map matching through noise and
sparseness"), solved with the Viterbi algorithm. leuvenmapmatching, Valhalla and GraphHopper use the same model.

| Part | Rule | Option |
|---|---|---|
| Candidates | the edges within `radius_m` of a GPS point, each at the point's projection on the edge | `radius_m` |
| Emission | a normal distribution of the distance from the point to the edge | `sigma_m`, the GPS error |
| Transition | an exponential distribution of the difference between the route distance along the edge directions and the straight distance between the two points | `beta_m` |
| Direction | a two-way road has two edges; the route distance selects the edge of the travel direction | |
| Break | when no candidate of a point can follow a candidate of the point before, the trace starts again at that point | |

The route between two points is the shortest path over the edges. A U-turn (an edge, then the other direction of the
same road) costs `uturn_m` (default 50 m) more, as in GraphHopper and Valhalla: without it, a noisy point next to a
short dead end gives a route into the dead end and back. Turn restrictions do not apply.

Monaco, driving, two routes (75 and 64 edges), a GPS point every 15 m, 20 traces of each with random noise
(`tests/test_mobility.py`):

| GPS noise (each axis) | Route edges found | Matched edges on the route | Traces with the exact route |
|---|--:|--:|--:|
| 3 m | 99.8 % | 99.4 % | 30 of 40 |
| 5 m | 98.3 % | 92.9 % | 8 of 40 |

The other edges come from a point near a junction at the start or the end (the edge before or after), and from short
roads next to the route.

### GPS-DTW: `method="dtw"`

`method="dtw"` matches with [GPS-DTW](https://khoshkhah.github.io/gps-dtw/) (the Rust crate gps-dtw, pinned in
`rust/duckosm-core/Cargo.toml`). It cleans the trace, interpolates it each second, and aligns it with points along
the roads by dynamic time warping. The turns come from `edge_graph`, so the turn restrictions apply. It needs the
`time` column: a time, or numbers of seconds. `sigma_m`, `beta_m` and `uturn_m` do not apply. The edge of each point
is the nearest edge of the route within `radius_m`.

Monaco, driving, the 600 traces of gps-dtw (50 shortest-path routes, GPS each 1, 5, 15 or 30 s, three noise levels),
one call for each method and interval:

| GPS interval | Route edges found, HMM | Route edges found, DTW | Matched edges on the route, HMM | Matched edges on the route, DTW | Time, HMM | Time, DTW |
|---|--:|--:|--:|--:|--:|--:|
| 1 s | 96.2 % | 94.5 % | 65.6 % | **97.4 %** | 2.4 s | **0.9 s** |
| 5 s | 96.2 % | 95.5 % | 90.1 % | **95.6 %** | 1.1 s | 1.1 s |
| 15 s | **94.9 %** | 92.2 % | 93.8 % | 93.9 % | **1.3 s** | 2.3 s |
| 30 s | **93.9 %** | 89.8 % | **95.1 %** | 91.2 % | **1.9 s** | 4.9 s |

Use `method="dtw"` for dense GPS (1 to 5 s), and the HMM (the default) for sparse GPS. The routes of this test are
shortest paths, which favour the HMM. A test on real trips can change the default.

## The edge id map

`edge_id` is the same after a rebuild for each edge whose OSM way and ends do not change ([stable edge
ids](../../concepts/edge-ids.md)). An OSM update or a new fix can split or join edges, and these get new ids.
`edge_id_map` maps each old edge:

| Old edge | Rows |
|---|---|
| its `edge_id` is in the new build | one row to itself, `share` 1 |
| split or joined | one row for each new edge of the same OSM way that lies on it in the same direction (within `tol_m`), with the share of its length (in tenths) |
| removed | no row |

```python
m = duckosm.mobility.edge_id_map(duckosm.open("old.duckdb").driving, net.driving)
counts = old_counts.merge(m, left_on="edge_id", right_on="old_edge_id")     # your data on the new edges
```

Notebook: [11. Mobility data](../../notebooks/11_mobility.md). Page: [Stable edge ids](../../concepts/edge-ids.md).
