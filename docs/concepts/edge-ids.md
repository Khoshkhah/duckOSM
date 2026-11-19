# Stable edge ids

Each edge has an `edge_id`, made from OpenStreetMap ids. It is not a row number.

```sql
edge_id = (hash(osm_id, source, target) >> 1)::BIGINT      -- DuckDB's hash()
```

| Input | Is |
|---|---|
| `osm_id` | the OSM way that the edge comes from |
| `source`, `target` | the OSM nodes where the edge starts and ends, in the direction of travel |

So:

| Rule | Why |
|---|---|
| **The two directions of a two-way road have two ids** | `source` and `target` change places, so the hash changes. Data for one direction (a count, a speed) joins on its own id |
| An unchanged edge keeps its id after a rebuild, in each area cut from the build, and in each export | the inputs are OSM ids, not row numbers |
| An id changes only when its edge changes | a new way id, new end nodes, or a new junction that splits the edge |
| The same road has the same id in each mode | the modes cut the ways at the same junctions |

## One id for each edge

`(osm_id, source, target)` must be unique. Where it repeats (a way that ends where it starts, or that runs between the
same two junctions twice), the build splits the edge at its middle with a **virtual node**. A virtual node has a negative
`node_id`, made from the content of the edge, so it is as stable as the rest.

## The same id in every mode

Each mode cuts the ways at the same points, `main.global_junctions`: each node that two highway ways share (a road and
a crossing footway too), and each end of a road. So a way that is in two modes has the same pieces and the same
`edge_id` in both. The same `edge_id` can then be in several schemas: across modes, the key is `(mode, edge_id)`.

## Merged edges

A chain of edges that is one road (the same `highway`, `name`, `oneway`, `maxspeed`, `lanes` and other tags) is
joined into one edge. Roads are not joined across OSM ways; chains of footways and paths are.

### `edge_id_map`

This table of each mode maps each piece to its merged edge:

| Column | Holds |
|---|---|
| `old_edge_id` | the `edge_id` of the piece, as in a build without merging |
| `new_edge_id` | the merged edge |
| `seq` | the position of the piece, 1 at the start |
| `is_reverse` | whether the piece runs against the drawing direction of its way |
| `osm_id` | the OSM way of the piece |

## Connector edges

An edge that joins a dangling path to the network has a negative `osm_id`: `-min(a, b)`, where `a` and `b` are the
`osm_id` of the path and of the road. Its `edge_id` uses the same formula. See
[Network clean-up](cleanup.md#connect-dangling-paths).

## Cuts and exports

A cut (`duckosm extract`) copies the edges as they are, so each `edge_id` is the same as in the larger database. The
[exports](../exports/index.md) keep `edge_id`: as the id (SUMO edge, MATSim link, GMNS `link_id`, OpenDRIVE road) or as
an attribute (GeoPackage, Lanelet2 `duckosm:edge_id`).

## Compute or match an id

Each database has the macro `edge_id_hash(osm_id, source, target)`:

```sql
SELECT count(*) FROM driving.edges WHERE edge_id <> edge_id_hash(osm_id, source, target);   -- 0
```

`hash()` belongs to DuckDB and can change in a new major version, so duckOSM stays on DuckDB 1.x. Outside DuckDB, join
your data on the three columns:

```sql
SELECT s.*, e.edge_id
FROM my_segments s JOIN driving.edges e USING (osm_id, source, target);
```
