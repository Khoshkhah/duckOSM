# GMNS crossings: where a zebra is painted, on which lanes, on which stretch of each

Status: **built** (2026-10-01, Kaveh: "go ahead with your recommendations": names `crossing` and `lane_crossing`; bus lanes covered, bike lanes not; signals alone paint
nothing). Branch: `gmns-values`, code in `src/duckosm/crossings.py`. Monaco: 741 crossings (499 from crossing ways, 242 from nodes), 909 lane rows.

## Problem

lanestyle draws zebra stripes from the `footway=crossing` ways by geometry: it clips each crossing's line to the road surface, drops
one that runs along the kerb, drops one that overlaps a longer one. That is a guess made in the reader, and the data cannot answer
what a zebra needs:

- **Which lanes it covers**: the driving lanes only (not a bike lane, a tram track, a parking lane), exactly those of the
  carriageway it crosses.
- **Which stretch of each lane**, so the stripes follow the lane (a curved road, a roundabout entry) instead of a straight line
  laid over it.
- **Crossings that are only a point**: Monaco has 639 `highway=crossing` nodes on roads, 279 (44 %) on no crossing way: a place,
  no line, so today they cannot be drawn.
- GMNS has no crosswalk: `link.facility_type` / `ped_facility` say sidewalk, not crossing.

## Proposal: two tables in `gmns_driving` (duckOSM extensions, like `lane_connector` and `curb_seg`)

### `crossing`: one row per crossing

| column | meaning |
|---|---|
| `crossing_id` | `<osm_id>` of the crossing's OSM way, or of its node when there is no way |
| `source` | `way` (a `footway=crossing` way: its line is the zebra's line) or `node` (a `highway=crossing` node on a road with no crossing way: the line is perpendicular to the road) |
| `crossing_type` | the OSM `crossing`: `zebra`, `marked`, `uncontrolled`, `traffic_signals`, `unmarked`, … |
| `markings` | the OSM `crossing:markings` (`yes`, `zebra`, `no`, …) |
| `width` | the zebra's width along the road, metres: OSM `width` if tagged, else 3 |
| `length` | across the road, metres, from the kerb edge of the first covered lane to the last |
| `geom` | the zebra's centre line, across the covered lanes only |

### `lane_crossing`: one row per crossing per lane it covers (the lane-level information)

| column | meaning |
|---|---|
| `crossing_id`, `lane_id`, `link_id` | which crossing, which lane (and its link) |
| `start_lr`, `end_lr` | the stretch of **this lane** the zebra covers, metres from the lane's start: `end_lr - start_lr` is the zebra's width along that lane |
| `across_from`, `across_to` | where this lane lies **across the zebra**, in the crossing's own frame: metres from the zebra's first edge to this lane's left and right edges. The stripes are laid in that one frame, so they stay continuous from lane to lane |

Only driving lanes (`allowed_uses` `auto`, and `bus`: a bus lane is a driving lane; bike lanes no) get a row. A crossing over a bike
lane is simply not on it.

**How a reader draws it.** For each `lane_crossing` row: cut the lane's own geometry between `start_lr` and `end_lr`, and in it paint the
stripes whose centre lies between `across_from` and `across_to` (pitch `stripe + gap` from `across_from = 0` of the crossing): each
stripe is a strip along the lane, as wide as `stripe` across it. No clipping to roads, no angle test, no overlap test: all of that
moved into the data.

## The shape: the best rectangle, then each lane's share of it (Kaveh, 2026-10-01: "find the best rectangle for the link; for more than one
link, the best for both; then project it on the lanes and find the best rectangle for each lane")

A parallelogram that follows the footway (tried first) staggered across lanes and looked wrong. A zebra is painted square to the traffic:

1. **One rectangle for the crossing**, for all the links it crosses: its axis the road's direction (the covered lanes' directions, opposite
   ones are one axis), `width` along it (the OSM `width`, else estimated from the width of the road crossed: 0.4 × it, between 2.5 and 4 m; Kaveh) centred on the middle of the crossing's stretch inside the road, and across
   it from the first covered lane's outer edge to the last one's (`crossing.geom`, `crossing.length`).
2. **Projected on each lane, as a real overlap:** the rectangle is intersected with each lane's own shape; `start_lr` .. `end_lr` is that
   overlap along the lane and `across_from` .. `across_to` its extent across the rectangle. A lane that is short or ends inside the zebra
   (Avenue Princesse Grace is a chain of 5-12 m links) still has its part; before, a lane needed both cuts of the rectangle on it and the
   middle lanes of such a road got no row.

## Rules (duckOSM, once, tested)

0. **Same level only** (Kaveh, 2026-10-01: stripes on a faded tunnel road): a crossing way paints only on lanes of its own level (`layer`, else
   bridge 1 / tunnel -1, else 0): 16 Monaco crossings had stripes on a tunnel, bridge or layered road.

1. **Crossing ways.** Intersect the way's line with each driving lane. Keep the crossing only if it lies across the road: 30 degrees or
   more from the lane direction, not winding. `start_lr` / `end_lr` come from that intersection, `across_*` from the position along the way.
2. **Crossing nodes with no crossing way.** The node is on a road way: project it onto the link, the line perpendicular to the
   link across all its driving lanes.
3. **Only driving lanes.**
4. **No duplicates.** A crossing way and the nodes at its ends are one crossing, not three; two crossings on one place are one, the
   longer wins.
5. A footpath or cycleway over a footway gets no row (no road lane to paint on).

Not changed: the `footway=crossing` links of `gmns_walking` stay (the pedestrian side: connectivity, routing).

## lanestyle

Reads `lane_crossing` and `crossing`, draws the stripes as above, as polygons on the lanes (stripe 0.5 m, gap 0.5 m, settings). The lane
lines stop at the zebra's footprint as now. The popup: the crossing's `crossing_id`, type, `source`. All the geometry guessing I added to
`render.py` (`_zebra_stripes`, `_across`) is deleted.

## Checks (Monaco)

- About 330 way crossings plus about 280 node crossings; rows per `source`, `crossing_type`; no row on a lane that is not `auto` / `bus`;
  every stretch inside its lane (0 <= `start_lr` < `end_lr` <= the lane's length).
- The picture: Avenue Princesse Grace, the Rond-Point du Sporting (no ring of stripes along a kerb), the point-only crossings square
  across their road, the stripes continuous across the lanes of a carriageway.
- Docs: `gmns_tables.md` (both tables), `docs/exports/gmns.md`, lanestyle's guide.

## Open questions

- Names: `crossing` and `lane_crossing`? (`crosswalk` instead of `crossing`?)
- A bus lane covered (my choice), or only `auto`?
- A signalised crossing with no markings gets a row but no stripes (`markings = no` or `crossing_type = traffic_signals` alone): my choice.
