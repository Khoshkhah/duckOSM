# Segment merging (`merge_segments`)

Contracts chains of consecutive road segments that are **the same street** into a single
edge, during graph simplification. Controlled by `options.merge_segments` (**default
`true`**; set `false` for a no-merge build).

## Why

The base simplifier (`GraphSimplifier`) splits every OSM *way* at junctions, but OSM
stores one physical street as **several way objects** — a new `osm_id` at each tag change
(bridge, surface, a relation boundary, or no reason at all). Two such ways meeting
end-to-end create a **degree-2 "through" node** that is not a real junction. `_find_junctions`
keeps it (any node shared by 2+ ways is a junction), so the street stays fragmented into
many short edges. On Södermalm ~56% of nodes were these spurious degree-2 nodes.

`merge_segments` removes them: where a chain of segments is unambiguously the same road,
it collapses them to one edge. (Södermalm: 4,568 → 2,803 edges, −39%.)

## What merges — the predicate

A node is a **contraction point** when *all* hold (see `_node2` in
`src/duckosm/processors/graph_simplifier.py`):

- it is a **real** node — `node_id > 0` (excludes the virtual midpoints created when
  self-loops are split, which must stay split);
- it has exactly **two** forward edges (degree 2);
- the two edges pass **through** it — one in, one out (`sum(fwd) = 1`); a one-way meeting
  itself head-to-head is not a through-node;
- the two edges agree on **every carried attribute**: `highway, name, oneway, maxspeed,
  lanes, surface, junction`.

The strict all-attributes rule (not just `highway`+`oneway`+`name`) is deliberate: it
guarantees the merged edge is **lossless** — no maxspeed/lane/surface change is silently
averaged away. It costs only a handful of merges versus the looser rule.

## Speed and lanes

`maxspeed` and `lanes` are both in the predicate, so a **change in either is a merge
boundary** — the road stays split where the speed limit or lane count changes, and the
merged edge always has one exact value for each. Nothing is averaged.

- **Speed.** Merging runs *before* `process_speeds`, so the predicate compares the **raw OSM
  `maxspeed` string** (`"50"`, `"30"`, `"50 mph"`, `null`). Two segments merge only if these
  are identical; a `50 → 30` transition is never collapsed. `process_speeds` then converts
  the merged edge's single `maxspeed` to `maxspeed_kmh`, and `calculate_costs` derives
  `cost_s` from it — so travel time stays correct over the whole merged edge.

- **Lanes.** A forward edge's `lanes` is the **per-direction** (forward) lane count
  (`lanes_fwd`; see the lanes mechanism). The predicate requires it equal along the chain, so
  a forward lane-count change is a merge boundary and the merged edge carries that one value.
  The **reverse** edge is built after the merge and takes its backward count (`lanes_bwd`)
  from the merged edge's representative (longest-member) `osm_id`. Edge case: if two merged
  segments share the same *forward* lanes but differ in *backward* lanes, the reverse edge
  reflects only the representative segment's backward count — tighten the predicate to also
  compare `lanes_bwd` if that matters for your use.

## The algorithm — maximal-chain contraction

Runs on the **forward edges only** (`simplified_edges_forward`), *after* self-loop
splitting and *before* reverse edges and the stable re-key are built — so reverse edges are
generated from the already-merged forwards, and the `edge_id` hash is assigned once at the
end. Steps (`_contract_chains`):

1. **`_inc`** — explode each forward edge into two half-edges `(node, other, …attrs)`.
2. **`_node2`** — group by node and keep the contraction points (predicate above).
3. **walk** (recursive CTE) — seed at each *terminal → contraction-point* boundary edge,
   then step through contraction points onto their other edge, accumulating the ordered
   node sequence `refs_acc` (oriented source→target, shared node dropped each step). A chain
   ends when it reaches a non-contraction node. Each chain is found from both ends; the
   `start < cur` filter keeps one. `edge_set` (the member ids) guards against revisiting.
4. **`_cmeta` / `_merged`** — for each chain, rebuild geometry and length from the stitched
   node coordinates (`raw.nodes`), carry the uniform attributes from any member, and set the
   representative `osm_id` (below).
5. **rebuild** `simplified_edges_forward` = untouched singleton edges + one edge per chain.

This contracts **every** maximal same-road chain, so the result is the minimum edge count
under the predicate; nothing mergeable is missed and re-running is a no-op (idempotent).

### `osm_id` and the stable `edge_id`

A merged edge spans several `osm_id`s. The stable id is
`edge_id = hash(osm_id, source, target, is_reverse)`, so the merged edge needs one `osm_id`:
it takes its **longest contributing member's** `osm_id` (`arg_max(osm_id, length_m)`). This
keeps the id scheme and the natural-key join intact with no schema change. (`refs` still
carries the full node list, so turn-restriction matching is unaffected.)

## The matching table — `<mode>.edge_id_map`

Written whenever `merge_segments` is on (empty if nothing merged). It lets a consumer keyed
on the pre-merge ids translate to the merged ids:

| column | meaning |
|--------|---------|
| `old_edge_id` | the segment's id in a **no-merge** build (`hash(osm_id, source, target, is_reverse)`) |
| `new_edge_id` | the id of the merged edge it became part of |
| `seq` | the segment's position along the road (forward direction, 1…k) |
| `is_reverse` | `false` = forward edge, `true` = the two-way reverse edge |

Both ids use the same stable hash the final graph carries. Edges **absent** from the table
are unchanged by the merge. After ComponentFilter, rows whose merged edge was dropped (a
small disconnected component) are pruned, so the table only references live edges.

## Complexity & optimality

- **Result-optimal:** merges every maximal same-road chain — minimum edges for the
  predicate, idempotent.
- **Performance:** ~O(E) for the typical case (OSM chains are short, ~2–5 segments). The
  recursive CTE copies the growing `refs_acc`/`edge_set` lists each step, so a chain of
  length `k` costs O(k²); each chain is also walked from both ends (~2×). Fine for short
  chains; a long split road (e.g. a motorway) is the bounded worst case.
- The strictly-optimal alternative is union-find connected-components on the contraction
  subgraph + one grouped aggregation (`ST_LineMerge` for geometry) → O(E·α(E)), no list
  copying, no double walk — at the cost of more complex ordered-`refs` stitching. Swap to it
  if the recursive walk proves slow at country scale.

## Config

```yaml
options:
  merge_segments: true     # default; false for a no-merge build
```

Only meaningful with `simplify: true` (it operates on the simplified forward edges). For a
`source.type: duckdb` clip it is inert — the clip inherits the parent build's already-merged
graph, so build the **parent** with `merge_segments` to get merged clips.

See also: [`docs/pipeline.md`](pipeline.md) (stages), [`docs/data_dictionary.md`](data_dictionary.md).
