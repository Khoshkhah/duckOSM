# GMNS lanes without netconvert: SimReady's lane rules

Status: implemented, 2026-11-23 (`gmns_lane_rules.rs`). The default stays netconvert when it is installed.

## The problem

`duckosm gmns` decides which turns are legal (`movement`, with the OSM turn restrictions) by itself. But it asks SUMO's
netconvert **which lanes** serve each turn and **the path** of each lane through the junction
(`gmns_sumo::lanes_from_sumo`). That has four costs:

- GMNS needs a SUMO install for its lanes. Without netconvert, the movements have no lanes and there is no `lane_connector`.
- netconvert is slow and large on large areas, and it cannot run in the browser (duckOSM Studio).
- The lane logic is SUMO's, not ours: we cannot read it, fix it or say why a lane goes where it goes.
- SimReady (the HD map, Level 4) has its own rules for the same question. Two lane logics give two answers.

## The decision

Port SimReady's lane-pairing rules into Level 3. Level 3 then owns the lane logic, and Level 4 reads it from GMNS.

| Part of SimReady | What it decides | Ported to | Not ported |
|---|---|---|---|
| `turn-rules`, rule 1 (turn kind) | `straight`, `slight_left`, `slight_right`, `left`, `right`, `u` from the angle of the two arms (15°, 45°, 150°) | yes | — |
| `turn-rules`, rule 3 (fork) | a diverge (one arm in, two or more out) or a merge (the reverse); the twins of a two-way road do not count | yes | — |
| `turn-rules`, allowed or not | — | no: GMNS's `movement` table already decides this, with the OSM restrictions | the whole decision |
| `connectors`, rules 2–5, 20, 24 (lane pairs) | the entering lanes of each turn, paired with the leaving lanes: from `turn:lanes`, else by position; a fork shares the lanes; a roundabout ring, entry and exit; bus lanes; bike lanes | yes, as `gmns_lane_rules.rs` | — |
| `connectors`, rules 6–9 (the path) | the curve of each lane pair, fitted to the kerb, with heights | no: that is Level 4 geometry | kerb fit, heights, sweep |

The source of the code: SimReady `rust/duckosm-sim/src/connectors.rs` and `turn_rules.rs` (the same owner). The port keeps the
rule numbers of SimReady's pages in comments, so the two can be compared line by line.

## What changes in GMNS

`lanes_from_sumo` has one seam: it gets from netconvert, for each turn (link in, link out), the list of lane pairs
(lane number in, lane number out). Everything after that — the movement rows for each run of lanes, the bike rows
(`<mvmt_id>-b<lane>`), `lane_check` — works from that list. The new code gives the same list from the rules:

| Step | netconvert (today) | rules (new) |
|---|---|---|
| lane pairs of each turn | netconvert's `<connection>` elements | `gmns_lane_rules::pairs` |
| movement lane ranges, bike rows, `lane_check` | the same code | the same code |
| `lane.geom_cut` (the lane ends at the junction) | netconvert's lane shape | the lane line cut at the junction radius (below) |
| `lane_connector.geom` (the path through the junction) | netconvert's internal lanes | a cubic Hermite curve from the cut end of the lane in to the cut start of the lane out, tangent to both lanes |

The junction radius of a node: half of the widest road at the node (the sum of the lane widths of its two directions), plus
1 m, at least 2 m; 0 at a node with only two arms where the road only continues. It is a simple first value. SimReady's
Level 4 replaces it with the real junction shape.

## The option

`duckosm gmns --lane-source auto|rules|netconvert` (`GmnsOptions::lane_source`):

- `netconvert`: as today.
- `rules`: SimReady's rules, no SUMO needed.
- `auto` (the default): netconvert when it is installed, else the rules. Today a GMNS build without netconvert has no lanes
  for its movements; with `auto` it has the rules' lanes.

The default stays netconvert until the comparison below shows that the rules are as good. Then `auto` becomes `rules`, and
netconvert stays as a check (`lane_check` lists the turns where the two disagree).

## The comparison

On Monaco, and then Kentucky, for each movement of `gmns_driving`:

| Measure | Pass |
|---|---|
| turns that get lanes | rules ≥ netconvert |
| turns whose lane pairs are the same | report the share; look at each difference by hand on Monaco |
| `lane_check` rows ("no way out", "no way in", "against its arrow") | rules ≤ netconvert |
| SUMO from the GMNS db (`duckosm sumo monaco_gmns.duckdb --netconvert`) | netconvert builds it with no error |

## Not in this step

- The rules for a junction of several nodes (SimReady joins a cluster of nodes into one junction). GMNS works node by node,
  as netconvert does without `junctions.join`.
- Lane changes along a link (lane sections, pockets that start inside a link): GMNS has the lanes at the end of each link.
- SimReady reading GMNS (its side of the change). That is SimReady's work, after this step.

## Result on Monaco (2026-11-23)

The same source db, `duckosm gmns -m driving`, once `--lane-source netconvert`, once `--lane-source rules`:

| Measure | netconvert | rules |
|---|---|---|
| time | 3.9 s | 1.2 s |
| lane pairs | 4,123 | 4,122; 4,020 (97.5 %) the same as netconvert's |
| turns with exactly the same lane pairs | — | 3,503 of 3,629 (96.5 %) |
| turns with no lane pair | 0 | 2 |
| `lane_check`: no way out / no way in | 14 / 0 | 5 / 4 |
| SUMO from the GMNS db (`duckosm sumo … --netconvert`) | builds | builds |
| lane connectors, median length | 6.3 m | 3.7 m (the simple junction radius) |

Two fixes came from this comparison: a merge orders its arms from the left (largest angle first, as SimReady), and a
movement keeps GMNS's own type (`left` and `right` from 30°), so the lanes follow the type of the movement.

The 126 turns that still differ are mostly cases where both are heuristics. The largest group (20 turns: two lanes go on
into one) is a case where the rules are better: both lanes go on, while netconvert ends one lane with no way out.

Next: the same comparison on Kentucky, then `auto` becomes `rules`, and netconvert stays as a check.

