# Stable edge ids

```sql
edge_id = (hash(osm_id, source, target) >> 1)::BIGINT      -- DuckDB's hash()
```

- **`osm_id`**: the OSM way the edge comes from.
- **`source`, `target`**: the OSM node ids where the edge starts and ends (or a
  [virtual node](#one-id-per-edge)). Their order is the direction, so the two directions of a
  two-way road have different ids.

`edge_id` is not a row number. All three inputs come from OpenStreetMap ids, so an unchanged edge gets
the same `edge_id` on every rebuild, in every area cut from the same build, and in every export. An
id changes only when its edge changes: a new way id, new end nodes, a new junction that splits it,
or, for a [merged edge](#merged-edges), a change in the pieces it is made of.

The `is_reverse` column says whether an edge runs against the way's drawing direction (a way tagged
`oneway=-1` is turned round first, so its one edge is not a reverse). For a merged two-way edge the
forward direction is chosen by node id, not by the drawing. It is not part of the id.

## One id per edge

`(osm_id, source, target)` must be unique. Where it would repeat (a way that ends where it starts, a
way that runs between the same two junctions twice, or goes from A to B and back), the build splits
the edge at its midpoint with a **virtual node**. A virtual node has `node_id < 0` and its id is a
hash of the edge's content, so it is as stable as the rest. If a repeated triple still survives, the
build stops with an error. Monaco: 1 virtual node in driving, 19 in walking, 21 in cycling.

## The same id in every mode

Every mode cuts ways at one shared set of points, `main.global_junctions`: each node of a road
that another highway way also uses (another road, or a footway, path or cycleway, such as a
crosswalk), and each node where a road ends. It is made once from all highway ways in `raw.ways`,
before any mode filters them. A road is cut at these points, and so is a footway or path that
passes through one, even in a mode that doesn't have the road. So a way that is in two modes is cut
into the same pieces and has the same `edge_id` in both. Monaco: 2,754 of the 2,765 driving edges
have the same `edge_id` in cycling, 1,882 in walking. The others are roads that mode doesn't keep.

So a crosswalk cuts the road it crosses in driving too, and the road cuts the crosswalk in walking
too (where a road without sidewalks isn't in the walking network): walkers and cyclists connect to
the road there, and the pieces are the same in every mode. Ways that don't exist on the ground
(`highway=proposed`, `construction`, …) cut nothing. `options.global_junctions: false` makes each mode
split at its own junctions, and the ids of a road can then differ between modes.

Because the mode is not in the id, the same `edge_id` can be in several schemas. Across modes, the key
is `(mode, edge_id)`.

## Merged edges

With `options.merge_segments` (on), a chain of edges that is one road is joined into one edge: the
edges must agree on `highway`, `name`, `oneway`, `maxspeed`, `lanes`, `surface`, `access`, `junction`,
`layer`, `bridge`, `tunnel` and `service`. Both directions take the `osm_id` of the piece at the
forward edge's `source` end (`seq` 1).

A point in `main.global_junctions` is never merged away (with `options.global_junctions` on). So
roads are not joined across OSM ways; only chains of footways, paths and the like are. Monaco:

| Mode | Edges with merging | Edges with `merge_segments: false` |
|---|---|---|
| driving | 2,765 | 2,765 |
| walking | 10,952 | 11,374 |
| cycling | 10,274 | 10,828 |

### `edge_id_map`

Each mode with merging has an `edge_id_map` table: one row per piece of each merged edge. The
reverse of a two-way road is a merged edge of its own, with its own rows. A piece and the merged
edge it maps to always run the same way.

| Column | Meaning |
|---|---|
| `old_edge_id` | the piece's `edge_id` in a build without merging |
| `new_edge_id` | the merged edge it is part of |
| `seq` | the piece's position, 1 at the forward edge's `source` end |
| `is_reverse` | whether the piece (`old_edge_id`) runs against its way's drawing |
| `osm_id` | the piece's own OSM way |

An edge that is not in the table was not merged. Rows of edges removed by the
[component filter](cleanup.md#component-filter) are removed too. Monaco, walking: 796 rows for
372 merged edges. The OSM ways of one merged edge, in order:

```sql
SELECT seq, osm_id FROM walking.edge_id_map WHERE new_edge_id = ? ORDER BY seq;
```

## Connector edges

The edges that join a dangling path to the network have a negative `osm_id`: `-min(a, b)`, where `a`
and `b` are the `osm_id`s of the path and of the road it joins. Their `edge_id` uses the same formula.
Select them with `osm_id < 0`. See [Network clean-up](cleanup.md#connect-dangling-paths).

## Clips and exports

A clip (`source.type: duckdb`, `build --source-db`, `duckosm extract`) copies edges as they are, so
every `edge_id` is the parent's. A clip has no `edge_id_map`. The road [exports](../exports/index.md)
keep `edge_id`: as the id (SUMO, MATSim, GMNS `link_id`, OpenDRIVE road id) or as an attribute or tag
(GeoPackage, Lanelet2 `duckosm:edge_id`). railML (rail) has none.

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
