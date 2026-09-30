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
| `note` | free text, for your own record |

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
