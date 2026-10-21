# Fix OSM errors

OpenStreetMap is sometimes wrong for a road you care about: a one-way street tagged as two-way, a
wrong lane count, a missing turn ban. The best fix is [editing OpenStreetMap](https://www.openstreetmap.org/edit),
but that takes time to reach your downloads. Meanwhile, duckOSM can apply the correction itself, on
every build, from a rules file.

## The rules file

```yaml
overrides:                   # fixes to a way (keyed by its OSM id)
  - osm_id: 4230100          # Avenue Delphine: make it one-way
    oneway: true
  - osm_id: 4230099          # Avenue Saint-Romain: not for bikes
    exclude_modes: [cycling]

turn_restrictions:           # turns OSM should forbid but doesn't
  - from_way: 4230100        # no turn from Avenue Delphine onto Avenue Saint-Romain
    via_node: 21924057
    to_way: 4230099
    restriction: no_left_turn
```

Give it to the build, on the command line or in your [config file](build.md#with-a-config-file):

```bash
duckosm build --pbf monaco-latest.osm.pbf -b monaco.geojson -m driving -m cycling --fixes fixes.yaml
```

```yaml
osm_overrides: fixes.yaml
```

The build log says how many rules matched, per network:

```text
OSM fixes (fixes.yaml, driving): 1 of 2 way rules matched this area
OSM fixes (fixes.yaml): 1 of 1 turn rule matched this area
OSM fixes (fixes.yaml, cycling): 2 of 2 way rules matched this area
```

On Monaco, the file above gives:

| | Without | With |
|---|---|---|
| driving edges of Avenue Delphine | 2 | 1 (one-way) |
| cycling edges of Avenue Saint-Romain | 18 | 0 |
| turns from Avenue Delphine onto Avenue Saint-Romain | 1 | 0 |

A rule for a way or junction that isn't in the area does nothing, so one file can hold fixes for
every area you build. Fixes are only applied when you name the file.

## Way fixes (`overrides`)

| Field | Does |
|---|---|
| `osm_id` | the way to fix (required) |
| `oneway` | `true` / `false`: force one-way or two-way |
| `lanes` | lanes per direction, both directions |
| `lanes_forward`, `lanes_backward` | lanes per direction, each separately |
| `layer` | vertical level (fixes a wrong `layer` tag, e.g. a ramp at ground level tagged `layer=-1`) |
| `exclude_modes` | remove the way from these networks: `driving`, `walking`, `cycling` |
| `remove` | `true`: remove the way from the build (no road, building or feature) |
| `tags` | set these OSM tags on the way before the build reads it, e.g. `{highway: unclassified}` for a car road that OSM tags `pedestrian` |
| `keep_nodes` | `[from, to]`: keep only this part of the way (OSM node ids). To keep the other part, add it under `new_ways` with `tags_from` |
| `turn_lanes_forward`, `turn_lanes_backward` | the painted arrows, in OSM `turn:lanes` syntax (`through;left\|right`) |
| `note` | free text, for your own record |

## New ways (`new_ways`)

A road that OSM does not have:

```yaml
new_ways:
  - id: 9000001049107
    source: nvdb:1049107
    nodes: [180929, [18.063835, 59.328511], 180933]
    tags: {highway: tertiary, oneway: "yes", name: "Rödbodtorget"}
```

| Field | Does |
|---|---|
| `id` | the way's id (required): 9,000,000,000,000 or more, used one time only. Do not change it: the edge ids come from it |
| `nodes` | the nodes in the way's direction: an OSM node id, or `[lon, lat]` for a new node. Make both ends OSM nodes, so that the way connects |
| `tags` | the OSM tags of the way |
| `tags_from` | an OSM way whose tags (before any fix) the new way starts with; `tags` goes on top |
| `source`, `note` | free text, for your own record |

A new way that ends on a node inside another way cuts that way there, so that way gets new edge ids.

## Turn fixes (`turn_restrictions`)

| Field | Does |
|---|---|
| `from_way`, `via_node`, `to_way` | the turn: from this way, through this junction node, onto that way (OSM ids, required) |
| `restriction` | `no_u_turn` (default), `no_left_turn`, `no_right_turn`, `no_straight_on`, or `only_*` |
| `note` | free text |

The turn disappears from the driving network everywhere: routing, and every export (SUMO, MATSim,
GMNS, OpenDRIVE). To find the ids, look the road up on [openstreetmap.org](https://www.openstreetmap.org)
(click it: the address ends in `way/<id>`; junctions are `node/<id>`), or use
[`duckosm way`](query.md#look-up-one-osm-way).

Only fix what you've checked on the ground or in imagery: a wrong `oneway` silently removes a
direction of travel.
