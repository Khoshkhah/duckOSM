# Network clean-up

OSM data has small gaps: a footway that ends a metre short of the road it meets, a road cut in two
by the area's border. One step deals with them, then an optional check.

| Step | Modes | Runs when |
|---|---|---|
| [Connect dangling paths](#connect-dangling-paths) | walking, cycling | `clip.connectivity_rescue` (on), in a build from a PBF |
| [Validation](#validation) | all | `validation.enabled` (off; on in the config template) |

## Connect dangling paths

A **dangling end** is a node that touches only one edge, where that edge is a `cycleway`, `footway`,
`path`, `steps`, `pedestrian` or `track`. For each one, the build adds a straight **connector edge**
to the nearest node within `clip.connect_snap_m` metres (default 10), in both directions. Without it,
the path would be a separate piece of the network.

A connector edge has:

- a negative `osm_id`, `-min(a, b)`: `a` and `b` are the `osm_id`s of the path and of the road at the
  node it joins. Select connectors with `osm_id < 0`.
- the `highway` of the path and `oneway = FALSE`; no `name`, `lanes` or other tags (NULL). In cycling, a
  connector that extends a [dismount](networks.md#dismount-edges) path is dismount too.
- an `edge_id` from the [usual formula](edge-ids.md).

Monaco:

| Mode | Dangling ends joined | Connector edges added |
|---|---|---|
| walking | 254 | 508 |
| cycling | 512 | 1,024 |

Limits: it joins the nearest *node*, not the nearest point of an edge, so a connector can be longer
than the real gap. The distance ignores levels, so it can join a path to a road on a bridge above
it. An end with no node within the distance is not joined. Driving isn't repaired.

## Component filter

Removed on 2026-11-14. The filter kept only the largest connected piece of each network and dropped the rest. But a
road that does not connect to the main network is still a real road, for example on an island, in a closed site, or a
road that the area's border cut off. Every road now stays. The settings `clip.keep_largest_component` and
`clip.min_component_edges`, and the option `--no-component-filter`, are still accepted so that old configs and scripts
work, but they do nothing.

## Where the border cuts

**Build from a PBF.** The PBF is cut to the boundary (strategy `smart` by default, `complete_ways` when
`options.build_features: false`; `options.clip_strategy` sets it): every way with a node inside
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

With `validation.enabled`, each mode is checked once it is built. A failed check stops the
build unless `validation.fail_on_error: false`. How to switch it on and what the output looks like:
[Check a build](../guides/check-build.md).

| Check (`validation.` key) | Passes when |
|---|---|
| `assert_single_component` | the largest piece holds at least 99.9% of the edges |
| `assert_no_stranded_named` | no edge with a name is outside the largest piece |
| `assert_unique_node_id` | no `node_id` appears twice in `nodes` |
| `assert_way_length_conserved` | no stretch of a kept OSM way is missing while an edge of the same way joins its two ends |
| `warn_layer_without_structure` | a warning only: edges with `layer` ≠ 0 but no `bridge` or `tunnel` tag |

The first two are skipped: a network can have separate pieces (see [Component filter](#component-filter)). `assert_way_length_conserved` is skipped in a
clip build, which has no `ways` table.
