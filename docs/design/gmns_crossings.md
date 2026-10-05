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

## One rectangle per road (Kaveh, 2026-10-03: "the interval is per road")

**Problem.** A crossing over a junction corner (Monaco `w1202361402`, lon 7.42457 lat 43.73882: one OSM way running across the corner of two roads, 28 degrees off
the one and 50 off the other) got ONE rectangle whose axis is the *average* of the covered lanes' directions: "an average of two roads points at neither". Both roads'
lanes are within 0.15 in sine of the crossing line, so both stay in the cover (rule 1 above only drops lanes far off the crossing line), the axis lies between them, and
the rectangle is skewed about 39 degrees to every lane it was given a row for (all 12 lanes of the two roads; the reader's 45 degree test (`abs(t . u) < 0.7`) lets them through).
lanestyle then lays the stripes in that tilted frame and they spill from the road the zebra is on into the neighbouring arm. In Monaco 27 of about 330 painted crossings have a
lane more than 20 degrees off their rectangle's axis.

**Rule.** The interval is per road. The covered lanes are grouped into roads by direction (axes within 30 degrees of each other, opposite directions are one axis: a dual
carriageway is one road, as before); **each road gets its own rectangle**, its axis the road's direction, its length across that road's lanes only, its width along it, centred on
that road's part of the crossing; and its own lane rows (`start_lr` / `end_lr` / `across_from` / `across_to`, which now all lie in that rectangle's frame). A crossing over a corner
is two zebras, each square to its road; a crossing over one road is one, as before.

**Table.** No new column or table: the first road keeps `crossing_id` (`w1202361402`), the others get `#2`, `#3` (the same scheme as two crossings of one OSM way). Each is a
`crossing` row with its own `geom` and `length` and its own `lane_crossing` rows. The overlap test between crossings (one already placed) is not applied between the roads of the same
crossing.

**Check.** A test with a skewed way across the corner of A (east-west) and B (north-south): two crossings, each rectangle axis-aligned with its road, no lane of B under A's.
Monaco: the number of painted crossings with a lane more than 20 degrees off the axis, before and after; pictures of `w1202361402` and two other of the 27.

## A zebra crosses a road, not a lane; a crossing way may cross two roads (2026-10-04, Kaveh)

Two changes in `build_crossings`, found at Avenue Prince Pierre in Monaco (the OSM crossing way `503475649`, lon 7.4181, lat 43.7327), where it crosses the two branches of a fork, `503475647` and `503475651`:

1. **The best lanes are chosen per piece of the line.** Where the line goes across several roads (the pieces of the line inside the lane surfaces), the lanes "most nearly perpendicular to the line" were chosen over all the pieces together. That rule is for a junction corner, where the line only touches the other road's lanes, and it threw away the whole second road (no zebra on `503475651#2f`). Now the choice is made inside each piece; a junction corner is still one piece. The crossing way gives two crossings, `w503475649` and `w503475649#2`, one rectangle per road.
2. **The rectangle spans every lane of the links it crosses**, also the lanes the line does not reach (an OSM crossing way can end inside the first lane): the lanes of a crossed link that lie within 3.5 m of the stretch of the line inside the road widen the rectangle across the road. Monaco: 497 crossings (2 more), 1,497 lane rows (16 more).

3. **A rectangle that lies mostly on another rectangle of the same crossing is dropped** (`_MAX_SAME_OVERLAP`, half of the smaller). A crossing line that touches a bend or a corner of a road gave two rectangles on the same ground (Rue Louis Notari, Monaco: 72 % of the smaller on the other); the old lane clip hid the double stripes, a reader that draws the rectangle shows them. The first rectangle already has a row for every lane it overlaps, so nothing is lost. Monaco: 8 overlapping pairs of rectangles
   (over 15 % of the smaller) become 2 (47 % and 25 %, true corners); 442 painted crossings, 1,481 lane rows.
