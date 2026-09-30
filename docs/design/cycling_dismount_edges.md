# Cycling dismount edges (stop deleting orphaned cycleways)

**Status:** implemented (2026-07-22, same day as proposed). `RoadFilter` (dismount WHERE branch
+ bidirectional oneway case), `DismountMarker` (the `dismount` column), `SpeedProcessor`
(walking-speed CASE), `FunctionalType` (`cycle_type='dismount'`); flag
`options.cycling_dismount`, default **on**; tests in `tests/test_cycling_dismount.py`.
Evidence measured on `sodermalm.duckdb`; missing-way tags verified against
`sweden-latest.osm.pbf` via `ST_READOSM`.

## Summary

The cycling graph silently loses real, legal cycle infrastructure. The road filter correctly
keeps every `highway=cycleway`, but fragments whose only connection to the rest of the network
runs through *non-cycling* edges (footway links, park connectors, boundary-cut approaches)
become islands in the cycling graph, and `ComponentFilter` sweeps them. The walking graph keeps
the same geometry, so the loss is invisible in any single-mode view.

Fix: include `footway` / `pedestrian` ways in the cycling graph as **dismount edges** — flagged
`dismount = TRUE`, costed at walking speed, bidirectional. This is the OSRM/Valhalla approach
("push your bike"): the graph stays legally correct (no riding on a gångbana), becomes fully
connected, and the orphaned cycleways survive `ComponentFilter` because their connectors now
exist. Gated by `options.cycling_dismount` (proposed default: **on**).

## Measured evidence (Södermalm)

Walking∖cycling diff on `edge_id` (global junctions make ids comparable across modes):

| missing from cycling | edges | km |
|---|---:|---:|
| `footway` | 3,587 | 99.7 |
| **`cycleway`** | **296** | **14.1** |
| `pedestrian` | 217 | 11.3 |
| `steps` | 595 | 7.3 |

The 296 cycleway edges come from 116 distinct `osm_id`s: 9 are synthetic (negative id); the
remaining **107 were all found in the source PBF, all `highway=cycleway`, none with
`bicycle=no` or `access=private/no`** — the filter keeps 107/107, so the loss happens after
filtering. The cycling graph totals 152.9 km; it is missing ~9 % of the area's dedicated cycle
infrastructure.

Legal context (Sweden): riding on a gångbana/footway is forbidden (children under 8 excepted),
so simply reclassifying footways as rideable would be wrong. *Walking* a bike is pedestrian
traffic and is always allowed — which is exactly what a dismount edge encodes.

## Spec

1. **Filter** (`road_filter.py`, cycling branch, only when `cycling_dismount`): the WHERE
   becomes `rideable_rules OR dismount_rules` with

   ```sql
   dismount_rules:
       highway IN ('footway', 'pedestrian')
       AND COALESCE(access, '') NOT IN ('private', 'no')
       AND COALESCE(foot,   '') <> 'no'
   ```

   `bicycle=no` does **not** exclude a dismount edge (pushing is walking); `access=private`
   still does. `steps` stays out in v1 (see open questions).

2. **`dismount` column** (BOOLEAN on `cycling.edges`, default FALSE): set in a post-pass after
   graph build, like `FunctionalType` — an edge is dismount when `highway IN ('footway',
   'pedestrian')` and its raw way's `bicycle` tag is not in `yes/designated/permissive`
   (join `raw.ways` on `osm_id`; synthetic/clip edges keep the value they carry). Footways
   that were already rideable via `bicycle=yes` stay `dismount = FALSE`.

3. **Speed → cost** (`speed.py`): the cycling branch becomes
   `SET maxspeed_kmh = CASE WHEN dismount THEN 5.0 ELSE 15.0 END`
   (`WALKING_SPEED` / `CYCLING_SPEED`). Cost calculation then needs no change at all.

4. **Oneway:** dismount edges are always bidirectional (a pushed bike is a pedestrian);
   ignore `oneway`/`oneway:bicycle` on them.

5. **`cycle_type`** (`functional_type.py`): dismount edges get the new enum value
   `'dismount'` instead of falling into `mixed_traffic`. Both columns exist deliberately:
   the boolean is the routing-critical flag — always present when the feature is on, and
   what `speed.py` keys costs off — while `cycle_type` is the functional classification
   (only built when `options.functional_types` is on), so classification consumers see
   dismount edges without joining anything.

6. **Ids:** additive and stable. Existing cycling `edge_id`s are content hashes and don't
   change; the new edges share their `edge_id` with their walking twins (global junction
   segmentation), which downstream cross-mode joins can rely on.

## Why this also fixes the orphan bug

`ComponentFilter` runs on the built mode graph. With dismount connectors present, the
previously-isolated cycleway fragments are part of the dominant component and survive — no
change to `ComponentFilter` itself, no size thresholds, no special cases.

## Interactions

- **Report/viz:** the per-mode highway breakdown picks the new rows up automatically; `viz.py`
  adds `dismount` to the tooltip columns when present (dashed styling = follow-up, not v1).
- **Exports** (`export-gis`, GMNS, …): verify the column passes through at implementation;
  GMNS `use` mapping for dismount edges should be the walk use, not bike.
- **Multimodal (`mm.*`):** cycling now overlaps walking on shared geometry; the mode-layered
  builder should be unaffected — add one assertion to the multimodal test.
- **Validation:** unchanged. `single_component`/`no_stranded_named` get *easier* to satisfy.

## Alternatives considered

- **Size-thresholded `ComponentFilter`** — keeps the fragments but they stay unroutable
  islands, and any threshold is arbitrary. Rejected.
- **Cross-mode connectivity glue** (count walking edges when deciding what's an island) —
  same flaw: keeps geometry that cycling routing still can't reach. Rejected.
- **Union with the walking graph at query time** — that's multimodal routing's job (`mm.*`),
  not the single-mode graph's. Out of scope.

## Expected effect (Södermalm)

Cycling gains ~111 km of dismount edges (99.7 footway + 11.3 pedestrian) and stops losing the
14.1 km of cycleway; total ≈ 278 km. Rebuild also picks up the `ST_LineSubstring` NaN fix
(current db predates it; 2 walking edges carry NaN coordinates).

## Test plan

- Unit: toy graph with a cycleway island connected only via a footway — flag on: island
  survives, connector is `dismount`, `maxspeed_kmh = 5`; flag off: current behaviour.
- Rebuild Södermalm: walking∖cycling diff shows **0 cycleway edges**; validators green.
- Multimodal build on a two-mode db passes.

## Open questions (for sign-off)

1. **Default on or off?** Proposed **on** — a routing graph that silently deletes 14 km of
   infrastructure is the worse default; `cycling_dismount: false` restores today's graph.
2. **`steps`?** Excluded in v1 (Södermalm alone has 1,190 step edges; pushing up stairs wants
   a penalty model, and ramps/wheeling channels aren't tagged reliably). Revisit if routing
   dead-ends at stair-only connections.
3. Is a flat 5 km/h enough, or do dismount edges want an extra fixed penalty (mount/dismount
   time) in cost calculation? v1: no extra penalty.
