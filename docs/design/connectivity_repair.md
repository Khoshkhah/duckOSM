# Path connectivity repair (`PathConnector`)

Reconnects **dangling cycleway / footway ends** to the road network so they survive the
component filter, in the **cycling** and **walking** graphs.

## The problem

OSM cycleways and footways are often drawn **crossing or meeting roads without a shared node** — the
path geometry ends a few metres from the road it should join, but no junction node ties them together.
So the pedestrian/cycle graph **fragments into many small disconnected components**, and
`ComponentFilter` (`keep_largest_component`) drops everything outside the largest one.

Measured on Tartu (before this step):

| mode | ways dropped (ways→edges) | of those, path/cycleway/footway |
|---|---|---|
| cycling | 843 / 7703 (11 %) | **549** |
| walking | 1106 / 11720 (9 %) | **433** |
| driving | 45 / 6817 | 0 |

Driving is barely affected (road networks are well-connected); the loss is almost entirely paths.

**Worked example.** Cycleway `osm 749048935` (`bicycle=designated`, `foot=designated`) is present in
`cycling.ways` / `walking.ways` but **absent from the edges** — it dangles ~0.5 m from `Kesk kaar` at
the `Kesk kaar`/`Tamme pst` junction with no shared node (its neighbour cycleway `749048934` ends 14 m
away, also unshared), so it forms an isolated component and is pruned.

## The fix

Add a synthetic **connector edge** from each dangling path end to the nearest node on the network,
*before* the component filter runs. The dangle then joins the largest component and is kept (and
becomes routable). This is the long-intended `connectivity_rescue` (`ComponentFilter` accepted the
flag as a no-op).

### Algorithm

Per mode (cycling / walking only):

1. **Find dangles.** A *dangle* = a node that touches **exactly one physical path segment** — i.e. a
   node incident to only one distinct undirected edge whose `highway ∈ {cycleway, footway, path,
   steps, pedestrian, track}`. (A path dead-end; roads and multi-way junctions are excluded.)
2. **Find the target.** For each dangle node `d`, the nearest **node** `n` within `snap_m` metres
   (default **10 m**) that is **not** on `d`'s own segment. (Nearest-node, not nearest-edge — see
   *Limitations*.)
3. **Add a connector.** Insert a straight edge `d → n` (and its reverse — paths are two-way), with the
   attributes below. `d` now shares node `n` with the network.

Runs **after `simplify_graph`** (edges are final & re-keyed) and **before `build_edge_graph`**, so the
connector is naturally part of the edge graph and the component filter sees the joined-up network.

### Connector edge attributes

| field | value |
|---|---|
| `osm_id` | **`-min(a, b)`** — where `a`, `b` are the osm_ids of the two roads being joined (the dangling path, and the road at the target node = the min osm_id of the segments touching that node). Negative marks it synthetic (`osm_id < 0`, never collides with real OSM ids, trivially filterable) **and points back to a real connected road** for traceability. No hashing needed. |
| `edge_id` | `(hash(osm_id, source, target) >> 1)::BIGINT` — same scheme as every other edge, so it's a normal stable id. (Two connectors can share an `osm_id` when they bridge the same pair of roads — they still get distinct `edge_id`s via their different `source`/`target`.) |
| `source` / `target` | dangle node / nearest node (and swapped for the reverse edge). |
| `geometry` | straight `LINESTRING(dangle_coord, nearest_coord)`. |
| `highway` | the dangling path's class (`cycleway` / `footway` / …) so it stays traversable and styles like a path. |
| marker | the **negative `osm_id` is the marker** — a connector is exactly `osm_id < 0`. No extra column, so the `edges` schema is unchanged whether or not this step ran. |
| `oneway` | `FALSE` (bidirectional, like the paths it joins). |
| `length_m` | metric length of the straight segment. |

**Why negate rather than reuse a real `osm_id` verbatim?** A positive `WHERE osm_id = 749048935`
would then return a segment that isn't part of that OSM way, breaking the "one positive `osm_id` → one
OSM way" assumption the rest of the stack (matchers, GMNS, exports) relies on. Negating keeps
connectors out of that positive namespace while still pointing back to a real road.

## Configuration (`clip` block)

```yaml
clip:
  keep_largest_component: true    # unchanged — still prunes what stays disconnected
  connectivity_rescue: true       # run PathConnector (cycling/walking) BEFORE the filter (default on)
  connect_snap_m: 10.0            # max dangle→node bridging distance, metres
```

`connectivity_rescue` already existed on `ComponentFilter` as an accepted-but-no-op flag (default
`true`); this step is its implementation. Set `connectivity_rescue: false` to keep builds as before.

## Limitations / future work

- **Nearest node, not nearest edge.** If the closest point on a road is mid-edge (no node there), we
  bridge to the road's nearest *node*, which can be a slightly longer/diagonal connector. A more exact
  version would project onto the edge and **split** it at the foot point. Fine for rendering and for
  routing reachability; revisit if geometric fidelity matters.
- **Still pruned if no node within `snap_m`.** Genuinely isolated paths (nothing within the tolerance)
  are still dropped — correct, they aren't reachable.
- **`snap_m` is a trade-off.** Too small misses real crossings (Tartu dangles sit 0.5–7 m from the
  road); too large can fuse unrelated parallel paths. 10 m is a safe default for urban data.
