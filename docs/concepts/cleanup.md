# Network clean-up

OSM data has small gaps: a footway that ends a metre short of the road it meets, a road cut in two
by the area's border. Two steps deal with them, then an optional check.

| Step | Modes | Runs when |
|---|---|---|
| [Connect dangling paths](#connect-dangling-paths) | walking, cycling | `clip.connectivity_rescue` (on) |
| [Component filter](#component-filter) | all | a boundary is set |
| [Validation](#validation) | all | `validation.enabled` (off; on in the config template) |

## Connect dangling paths

A **dangling end** is a node that touches only one edge, where that edge is a `cycleway`, `footway`,
`path`, `steps`, `pedestrian` or `track`. For each one, the build adds a straight **connector edge**
to the nearest node within `clip.connect_snap_m` metres (default 10), in both directions. Without it,
the path would be a separate piece and the component filter would drop it.

A connector edge has:

- a negative `osm_id`, `-min(a, b)`: `a` and `b` are the `osm_id`s of the path and of the road at the
  node it joins. Select connectors with `osm_id < 0`.
- the `highway` of the path, no `name`, and `oneway = FALSE`.
- an `edge_id` from the [usual formula](edge-ids.md).

Monaco:

| Mode | Dangling ends joined | Connector edges added | Left after the component filter |
|---|---|---|---|
| walking | 310 | 620 | 568 |
| cycling | 555 | 1,110 | 1,008 |

Limits: it joins the nearest *node*, not the nearest point of an edge, so a connector can be longer
than the real gap. The distance ignores levels, so it can join a path to a road on a bridge above
it. An end with no node within the distance is not joined. Driving isn't repaired.

## Component filter

The filter keeps the largest connected piece of each network and drops the rest: stubs cut off by
the border and small isolated pieces. It counts pieces by edges, and ignores direction (weakly
connected). Dropped edges also leave `edge_graph`, `turn_restrictions`, `nodes` and `edge_id_map`.

| Key | Default | Does |
|---|---|---|
| `clip.keep_largest_component` | `true` | keep only the largest piece |
| `clip.min_component_edges` | `1` | with `keep_largest_component: false`, also keep every piece with at least this many edges |
| `clip.strongly_connected` | `false` | not implemented: `true` logs a warning and the filter works as above |

It runs only when a boundary is set (a whole country has real islands) and, for a PBF build, when
the edge graph is built. It runs in clip builds too. Monaco:

| Mode | Edges dropped | Pieces dropped |
|---|---|---|
| driving | 77 | 15 |
| walking | 616 | 101 |
| cycling | 447 | 80 |

Because direction is ignored, the kept network can still hold a one-way dead end: an edge you can
reach but not leave.

## Where the border cuts

**Build from a PBF.** The PBF is cut with `osmium` (`complete_ways`): every way with a node inside
the boundary is kept whole, so roads that cross the border stick out past it. Then the component
filter removes what doesn't connect. `boundary.buffer_m` grows the boundary first.

**Clip from a parent build.** `clip.predicate` decides which edges are copied:

| `clip.predicate` | Keeps an edge when |
|---|---|
| `intersects` (default) | it touches the boundary: roads that cross the border are kept whole |
| `within` | it lies completely inside: no stubs, but roads that cross the border are lost |
| `centroid` | its middle point is inside |

`duckosm extract` always uses `intersects`.

## Validation

With `validation.enabled`, each mode is checked at the end of the build. A failed check stops the
build unless `validation.fail_on_error: false`. How to switch it on and what the output looks like:
[Check a build](../guides/check-build.md).

| Check (`validation.` key) | Passes when |
|---|---|
| `assert_single_component` | the largest piece holds at least 99.9% of the edges |
| `assert_no_stranded_named` | no edge with a name is outside the largest piece |
| `assert_unique_node_id` | no `node_id` appears twice in `nodes` |
| `assert_way_length_conserved` | no stretch of a kept OSM way is missing while an edge of the same way joins its two ends |
| `warn_layer_without_structure` | a warning only: edges with `layer` ≠ 0 but no `bridge` or `tunnel` tag |
| `assert_edge_id_stable` | not implemented: reports "skipped" |

The first two run only when the component filter ran. `assert_way_length_conserved` is skipped in a
clip build, which has no `ways` table.
