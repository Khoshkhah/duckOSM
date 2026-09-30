# Stable edge ids

```sql
edge_id = (hash(osm_id, source, target) >> 1)::BIGINT      -- DuckDB's hash()
```

- **`osm_id`**: the OSM way the edge comes from.
- **`source`, `target`**: the OSM node ids where the edge starts and ends. Their order is the
  direction, so the two directions of a two-way road have different ids.

`edge_id` is not a row number. All three inputs are OpenStreetMap ids, so an unchanged edge gets the
same `edge_id` on every rebuild, in every area cut from the same build, and in every export. An id
changes only when its edge changes: a new way id, new end nodes, or a new junction that splits it.

The `is_reverse` column says whether an edge runs against the way's drawing direction. It is not
part of the id.

## One id per edge

`(osm_id, source, target)` must be unique. Where it would repeat (a way that ends where it starts, a
way that runs between the same two junctions twice, or goes from A to B and back), the build splits
the edge at its midpoint with a **virtual node**. A virtual node has `node_id < 0` and its id is a
hash of the edge's content, so it is as stable as the rest. If a repeated triple still survives, the
build stops with an error. Monaco: 5 virtual nodes in driving, 23 in walking, 21 in cycling.

## The same id in every mode

Every mode splits roads at one shared set of points, `main.global_junctions`: each node where two
road ways meet, or a road ends. It is made once from all OSM ways, before any mode filters them.
So a road that is in two modes is cut into the same pieces and has the same `edge_id` in both.
Monaco: 1,997 of the 2,133 driving edges have the same `edge_id` in cycling, 1,371 in walking.
The others are roads that mode doesn't include.

Footways, paths, cycleways, steps, pedestrian streets, bridleways and corridors don't split a road
they touch; they split only themselves. `options.global_junctions: false` makes each mode split at
its own junctions, and the ids of a road can then differ between modes.

Because the mode is not in the id, the same `edge_id` can be in several schemas. Across modes, the key
is `(mode, edge_id)`.

## Merged edges

With `options.merge_segments` (on), a chain of edges that is one road is joined into one edge: the
edges must agree on `highway`, `name`, `oneway`, `maxspeed`, `lanes`, `surface`, `access`, `junction`,
`layer`, `bridge`, `tunnel` and `service`. The merged edge takes the `osm_id` of its first piece, at
its `source` end.

A point in `main.global_junctions` is never merged away. So roads are not joined across OSM ways;
only chains of footways, paths and the like are. Monaco:

| Mode | Edges with merging | Edges with `merge_segments: false` |
|---|---|---|
| driving | 2,133 | 2,133 |
| walking | 9,232 | 10,298 |
| cycling | 8,440 | 9,778 |

### `edge_id_map`

Each mode with merging has an `edge_id_map` table: one row per piece of a merged edge, and one more
for the reverse direction of a two-way road.

| Column | Meaning |
|---|---|
| `old_edge_id` | the piece's `edge_id` in a build without merging |
| `new_edge_id` | the merged edge it is part of |
| `seq` | the piece's position, 1 at the forward edge's `source` end |
| `is_reverse` | the direction |
| `osm_id` | the piece's own OSM way |

An edge that is not in the table was not merged. Rows of edges removed by the
[component filter](cleanup.md#component-filter) are removed too. Monaco, walking: 1,794 rows for
738 merged edges. The OSM ways of one merged edge, in order:

```sql
SELECT seq, osm_id FROM walking.edge_id_map WHERE new_edge_id = ? ORDER BY seq;
```

## Connector edges

The edges that join a dangling path to the network have a negative `osm_id`: `-min(a, b)`, where `a`
and `b` are the `osm_id`s of the path and of the road it joins. Their `edge_id` uses the same formula.
Select them with `osm_id < 0`. See [Network clean-up](cleanup.md#connect-dangling-paths).

## Clips and exports

A clip (`source.type: duckdb`, `build --source-db`, `duckosm extract`) copies edges as they are, so
every `edge_id` is the parent's. A clip has no `edge_id_map`. Every [export](../exports/index.md)
uses `edge_id` as the id in its own format.

## Compute or match an id

Every database has two macros:

| Macro | Formula |
|---|---|
| `edge_id_hash(osm_id, source, target)` | the formula above |
| `edge_id_hash_v1(osm_id, source, target, is_reverse)` | the older formula, which also hashed `is_reverse`: to map ids from a database built with it |

```sql
SELECT count(*) FROM driving.edges WHERE edge_id <> edge_id_hash(osm_id, source, target);   -- 0
```

In Python: `from duckosm import edge_id_hash, edge_id_expr` (one id, or the SQL expression).

`hash()` is internal to DuckDB and may change in a new major version, so duckOSM requires
`duckdb<2`. Outside DuckDB the id can't be recomputed. To give your own data the ids, join on the
three columns and read `edge_id`:

```sql
SELECT s.*, e.edge_id
FROM my_segments s JOIN driving.edges e USING (osm_id, source, target);
```
