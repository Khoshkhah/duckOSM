# Functional edge types — `walk_type` and `cycle_type`

**Status:** implemented (branch `edge-id-segmentation-overhaul`; config `functional_types`, default
on) as a post-process (`FunctionalType`) that joins each edge to its raw way tags — no simplifier
plumbing. Adds one derived enum column to the `walking` and `cycling` `edges` tables.

## Summary

Walking and cycling edges already carry a type — the `highway` column — but it is **too coarse for
routing, rendering, and multimodal logic**, because the mode-critical distinction lives in an OSM
*sub-tag* (`footway=*`, `cycleway:<side>=*`) that never reaches the `edges` table. On `edges`, a
sidewalk, a road crossing, and a standalone footpath all read `highway=footway` and are
indistinguishable; a protected cycle track, a painted lane, and a sharrow all read
`highway=residential`.

This doc proposes a single derived column per mode:

- `walking.edges.walk_type`
- `cycling.edges.cycle_type`

that collapses the OSM tag soup into a clean, stable functional enum. `highway` stays unchanged
(OSM-faithful physical class); the new column adds the *function*. This is the pedestrian/cyclist
analog of GMNS `lane.allowed_uses` (see `docs/gmns_export.md`).

## The gap (measured on `sodermalm_pbf`)

`highway` on the mode edges is present but blunt. The sub-type that matters is only in `raw.ways.tags`:

```
walking.edges.highway:  footway 7464 · residential 2130 · steps 1190 · service 1166 · …
  but footway=* (in raw tags only):  <generic> 1745 · crossing 258 · sidewalk 178 · traffic_island 3 · link 2
        → 258 crossings + 178 sidewalks + 1745 footpaths all collapse to "footway" on edges

cycling.edges.highway:  residential 1642 · service 830 · cycleway 749 · tertiary 232 · …
  but cycleway:right=* (in raw tags only):  separate 93 · lane 73 · no 69 · track 32 · share_busway 3 · shared_lane 1
        → a protected track, a painted lane, and mixed traffic all collapse to "residential" on edges
```

Neither the `edges` table nor the `{mode}.ways` table (which keeps `tags`) exposes these as a
queryable, routable column. A router or renderer has to re-parse `raw.ways.tags` and re-apply the
directional side logic every time.

## `walk_type`

One value per walking edge, derived from `highway` + `footway` + `conveying` + `area` + `indoor`.

| `walk_type` | derived from | meaning |
|---|---|---|
| `sidewalk` | `footway=sidewalk` (or a road carrying a `sidewalk` attribute) | walkway alongside a road |
| `crossing` | `footway=crossing` | pedestrian crossing of a road — the "cross here" link between two sides |
| `footpath` | `highway=footway` (generic) | standalone dedicated footway |
| `steps` | `highway=steps` | stairs |
| `escalator` | `highway=steps`/`footway` + `conveying` | escalator / moving walkway (often one-way) |
| `pedestrian_street` | `highway=pedestrian`, not `area=yes` | pedestrianised street |
| `plaza` | `highway=pedestrian` + `area=yes` | open pedestrian square |
| `shared_street` | `highway=living_street` | peds + slow cars |
| `shared_road` | `highway ∈ {residential, service, unclassified}` | road centerline pedestrians share |
| `corridor` | `highway=corridor` or `indoor=yes` | indoor passage |
| `platform` | `highway=platform` | transit platform |
| `path` | `highway ∈ {path, track, bridleway}` | informal / rural shared path |

Reference derivation (SQL, evaluated in the walking build):

```sql
CASE
  WHEN highway = 'steps'  AND tags['conveying'] IS NOT NULL          THEN 'escalator'
  WHEN highway = 'steps'                                             THEN 'steps'
  WHEN highway = 'footway' AND tags['footway'] = 'sidewalk'          THEN 'sidewalk'
  WHEN highway = 'footway' AND tags['footway'] = 'crossing'          THEN 'crossing'
  WHEN highway = 'footway'                                           THEN 'footpath'
  WHEN highway = 'pedestrian' AND tags['area'] = 'yes'               THEN 'plaza'
  WHEN highway = 'pedestrian'                                        THEN 'pedestrian_street'
  WHEN highway = 'living_street'                                     THEN 'shared_street'
  WHEN highway IN ('residential','service','unclassified')           THEN 'shared_road'
  WHEN highway = 'corridor' OR tags['indoor'] = 'yes'               THEN 'corridor'
  WHEN highway = 'platform'                                          THEN 'platform'
  WHEN highway IN ('path','track','bridleway')                      THEN 'path'
  ELSE 'footpath'
END AS walk_type
```

## `cycle_type`

One value per cycling edge, derived from `highway`, `segregated`, and the **directional** cycle
infrastructure tag `cycleway:<side>`.

| `cycle_type` | derived from | meaning |
|---|---|---|
| `cycleway` | `highway=cycleway` | dedicated off-road bike path |
| `cycle_track` | `cycleway:<side>=track` | physically separated track alongside a road |
| `cycle_lane` | `cycleway:<side>=lane` | painted on-road lane |
| `shared_lane` | `cycleway:<side>=shared_lane` | sharrow — marked but shared with cars |
| `bus_cycle_lane` | `cycleway:<side>=share_busway` | shared bus + bike lane |
| `segregated_path` | `highway ∈ {path,track,bridleway}` + `segregated=yes` | foot+bike path, sides separated |
| `shared_path` | `highway ∈ {path,track,bridleway}` (+ `bicycle=designated`) | mixed foot+bike path |
| `mixed_traffic` | a road with no cycle infra of its own (incl. `cycleway=separate/no`) | cyclist shares the carriageway |

Two subtleties, both parallel to earlier data-model notes:

1. **`cycle_type` is directional** — like `lanes`, the relevant side depends on the direction of
   travel. A forward edge (`is_reverse = FALSE`) reads `cycleway:right`; the reverse edge reads
   `cycleway:left`. (`cycleway:both` / `cycleway` apply to both.) So the same physical road can be
   `cycle_lane` forward and `mixed_traffic` backward.

2. **`separate` = a separately-mapped track.** `cycleway:right=separate` means the cycle track is
   drawn as its **own** `highway=cycleway` way (a distinct `osm_id`). On the road edge that resolves
   to `mixed_traffic`; the track itself is a separate `cycleway` edge. This is the same
   attribute-model vs separate-way split as sidewalks — the separate track cannot share the road's
   `edge_id` and must be spatially associated if a link is wanted.

Reference derivation (SQL; `cw_side` = the side matching the edge's direction):

```sql
-- cw_side := is_reverse ? tags['cycleway:left'] : tags['cycleway:right']
--            (fall back to tags['cycleway:both'] / tags['cycleway'])
CASE
  WHEN highway = 'cycleway'                                          THEN 'cycleway'
  WHEN cw_side = 'track'                                             THEN 'cycle_track'
  WHEN cw_side = 'lane'                                              THEN 'cycle_lane'
  WHEN cw_side = 'shared_lane'                                       THEN 'shared_lane'
  WHEN cw_side = 'share_busway'                                      THEN 'bus_cycle_lane'
  WHEN highway IN ('path','track','bridleway') AND tags['segregated'] = 'yes' THEN 'segregated_path'
  WHEN highway IN ('path','track','bridleway')                      THEN 'shared_path'
  ELSE 'mixed_traffic'   -- includes cycleway:<side> = separate / no / NULL
END AS cycle_type
```

## Structural modifiers (orthogonal)

These are separate from the functional type and largely **already** available as columns
(`bridge`, `tunnel`), or should be surfaced alongside:

- `bridge=yes` → footbridge / cycle bridge  *(already a column)*
- `tunnel=yes` → underpass  *(already a column)*
- `indoor=yes` / `covered=yes` → indoor / covered
- `conveying` → escalator / moving walkway (folded into `walk_type='escalator'` above)
- steps extras: `incline`, `handrail`, `step_count`, `wheelchair`, `ramp`
- crossing sub-kind: `crossing=traffic_signals|marked|unmarked|zebra` (refines `walk_type='crossing'`)

Keep these as their own attributes; do not overload the functional enum with structure.

## Where it slots into the build

- Computed in `RoadFilter` (mode = walking / cycling), where the raw `tags` MAP is still in scope,
  and carried through `GraphBuilder` / `GraphSimplifier` onto `edges` like `highway`/`lanes`.
- Add columns to `docs/data_dictionary.md` under the `<mode>` `edges` table.
- `cycle_type` must be computed **per directed edge** (it reads the side that matches `is_reverse`),
  the same way `lanes` uses `lanes_fwd` / `lanes_bwd`.

## Why it's worth it

- **Routing cost.** Crossings carry signal wait + exposure; steps block wheelchairs; escalators are
  one-way; a protected `cycle_track` is safer/faster to weight than `mixed_traffic`. Today the router
  can't see any of this — every `footway` and every `residential` looks identical.
- **Rendering.** Sidewalks, crossings, paths, protected tracks, and painted lanes should style
  differently; right now they are one blob per `highway` value.
- **Multimodal / side-accurate structure.** `walk_type='crossing'` edges are literally the links
  between the two sides of a street, and `walk_type='sidewalk'` are the through-edges — the exact
  scaffolding needed for side-accurate pedestrian routing and for placing multimodal transfers
  realistically (see `docs/multimodal.md`).

## Non-goals / open questions

- **Not** a geometry change — no per-side sidewalk/track edges are created here; this only *labels*
  the existing centerline edges. Per-side geometry is a separate, larger piece of work.
- Linking a **separately-mapped** sidewalk/cycle track (`separate`) to its parent road needs a
  spatial association, not this column.
- Should `walk_type='shared_road'` further split by the parent road class (a `service` alley vs a
  `residential` street), or is `highway` enough alongside it? (Proposed: keep it in `highway`.)
- Coverage: `footway=*` and `cycleway:<side>=*` are sparsely tagged in some regions — the enum
  degrades gracefully to `footpath` / `mixed_traffic`, but a coverage report per area would help
  decide where the column is trustworthy (cf. `docs/known_osm_issues.md`).
