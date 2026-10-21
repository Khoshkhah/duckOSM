# OSM fixes: new ways and way cuts

**Status:** approved by Kaveh and implemented 2026-10-20 (`DuckOSM._raw_way_fixes`, tests in `tests/test_osm_overrides.py`).
Kaveh's changes to the proposal: the id comes from the NVDB element (a convention of his rules file, not of the library), and
Rödbodtorget gets only branch B.

**Result.** Norrmalm: `driving.edges` has 3 new edges (Rödbodtorget 1, the Vasabron link 2) and has 4 fewer edges on way `40274443`.
Tegelbacken edge `4271686175640876552` keeps its id. Every driving edge has a row in `visualization.edge_levels`.

## Problem

An `osm_overrides.yaml` rule changes one OSM way that exists, always all of it. It cannot:

1. **Add a way** that OSM does not have. Example: Rödbodtorget in Norrmalm. NVDB has it (tertiary, one-way north, about
   70 m, two branches from the Fredsgatan west end to Tegelbacken). OSM has no way there.
2. **Change only part of a way.** Example: way `40274443` (known_osm_issues #17). Its east part, `1740740220` → `180929`, is a
   car road (NVDB segment `3678314`). Its west part, `266272124` → `119663` → `1740740220` (15.5 m), is a cycle path only. A rule
   for the car road makes the whole way a car road, so the west part gets 2 car edges that NVDB does not have.

## Design

Two additions to `osm_overrides.yaml`. The step `DuckOSM._raw_way_fixes` applies both, after `load_pbf` and before all other
steps, as it applies `remove` and `tags`. Every later step (global junctions, RoadFilter, `features.*`, exporters) then sees a
normal OSM way.

### 1. `new_ways:` (a top-level list)

```yaml
new_ways:
  - id: 9000001049107          # required, unique, never changed
    source: nvdb:1049107,1049108,1049109,1049110
    nodes: [180929, [18.063835, 59.328511], [18.063710, 59.328759], 180933]
    tags: {highway: tertiary, oneway: "yes", name: "Rödbodtorget"}
    note: "..."
```

- **`id`**: a positive number of 9,000,000,000,000 (`NEW_WAY_ID_MIN`) or more. OSM way ids are below 2,000,000,000 (Norrmalm: at
  most 1,511,091,809). Negative ids are not possible: PathConnector uses them, and GMNS reads `abs(osm_id)` as the OSM way of an
  edge. The build stops with an error if an id is below 9,000,000,000,000 or is used two times.
  Convention in Kaveh's rules file: id = 9,000,000,000,000 + the NVDB element id that the way comes from. The library does not
  check this convention, because NVDB is a Swedish source and other areas use other sources.
- **`source`**: free text, where the way comes from (`nvdb:<id>`). The build does not read it.
- **`nodes`**: in the direction of the way. An item is an OSM node id, or `[lon, lat]` for a new node. A new node gets the id
  `id * 1000 + position` (for example 9000001049107002), so it is stable and is far above the OSM node ids (Norrmalm: at most
  13,849,437,225). The first and the last item must be OSM node ids, so that the way connects to the network. If an end is a new
  node, the build logs a warning (the way is a dead end).
- **`tags`**: OSM tags, as on a real way. **`tags_from: <osm_id>`**: the way starts with the OSM tags of that way, before any fix;
  `tags` then goes on top.
- The edge ids are stable, because `edge_id` is `hash(osm_id, source, target)` and the rule gives all three.

### 2. `keep_nodes: [from, to]` (a field of an `overrides` rule)

The way keeps only its nodes from `from` to `to`. The build drops the other nodes from the way. To keep the dropped part, add it as
a `new_ways` item with `tags_from: <osm_id>`.

Which part keeps the OSM id: the part that OSM has right keeps it, and the part that OSM does not have becomes the new way. Then
the OSM way keeps its OSM meaning, its edge ids stay, and the new way maps to one source element. Way `40274443`:

```yaml
overrides:
  - osm_id: 40274443           # the cycle path, as in OSM
    keep_nodes: [266272124, 1740740220]
new_ways:
  - id: 9000003678314          # the car road, NVDB segment 3678314
    source: nvdb:3678314
    nodes: [1740740220, 1740740223, 180929]
    tags_from: 40274443
    tags: {highway: unclassified, oneway: "yes", lanes: "2", "oneway:bicycle": "no"}
```

## Effects

- **Edge ids of other ways.** A new way that ends on an inner node of a way cuts that way at the node (global junctions). Then
  the cut way gets new edge ids. Norrmalm: OSM nodes `180929` and `180933` are junctions already, so no way is cut there. Branch A
  of Rödbodtorget would end at `12957251840`, an inner node of Tegelbacken edge `4271686175640876552`. That is why it is left out.
- **The cut way.** Its other part gets new edge ids in every mode (another osm_id). Way `40274443`: the 2 cycle-path edges of the
  east part changed ids in walking and cycling.
- **Clip builds** (`source.type: duckdb`) copy the rows of the parent build, so they get the new ways from the parent.
- **`features.*` and maps** show the new way as a road.

## Rödbodtorget

NVDB (`fetching-sweden-data`, `nvdb.road_network`, 2026-10-20) has two one-way branches. Both start at the NVDB point
18.06372, 59.32839, which is 12.8 m west of OSM node `180929` on way `40274443`.

| Branch | NVDB segments | NVDB end | OSM end node | Built |
|---|---|---|---|---|
| A (west) | 3678311, 3929353, 3678312 (76 m) | 18.06311, 59.32893 | `12957251840` on Tegelbacken, 22.8 m from the NVDB end | no |
| B (east) | 1049107, 1049108, 1049109, 1049110 (69 m) | 18.06352, 59.32895 | `180933` (Jakobsgatan, Tegelbacken, Rödbodgatan), 1.9 m from the NVDB end | yes, way `9000001049107` |

Branch B starts at `180929`, 12.8 m from the NVDB start. To start at the NVDB point, a rule must add a node into way `40274443`.
That is a third feature, and this design does not include it.

## Tests

- `test_keep_nodes_and_new_ways`: a way cut, a new way with OSM nodes and a `[lon, lat]` node, `tags_from` with `tags` on top.
- `test_new_way_id_below_the_range_or_used_twice_stops_the_build`.
- The Norrmalm rebuild: the new edges, the edge ids that changed, and `visualization.edge_levels`.
