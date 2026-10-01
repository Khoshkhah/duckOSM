# GMNS lanes along a run of pieces: one road, not many pieces

**Status:** approved by Kaveh and built 2026-10-01 (branch `lane-runs`: `_chain_runs`,
`_run_lane_wkts`, `_build_lane_connectors`). Follows
[gmns_lane_connectors.md](gmns_lane_connectors.md); Kaveh: the Tunnel Dorsale sample
(`2949438343165019057_1 > 6486080469490879724_1`) "was one sample of the whole unsmooth connection
between road pieces". Checked on Monaco only.

## Problem

duckOSM cuts an OSM way into pieces at every junction, and `_build_lane_curb` places each piece's
lanes **on its own**: its own pairing decision (`_paired_gaps`), its own offset curve, its own ends.
A road is a chain of such pieces, and where two pieces meet the lanes often don't:

| Where a lane continues into the next piece of the same road (Monaco, 1,612 cases) | with a connector (a jump over 0.3 m) |
|---|---|
| through a junction: the lanes were cut there (`_build_lane_connectors`) and joined back by a straight connector; seams, broken lane lines | 577 |
| the two pieces placed at different offsets: one paired with an opposite carriageway, the next (too short to judge, 4.3 m) not; or `placement` | 55 |
| a bend: each piece offset on its own, a wedge on the outer side | 46 |

Also 288 lanes are shorter than 2 m after trimming: a 4.3 m piece cut at both ends leaves 0.7 m
stubs whose edge lines fan out like spokes (the Tunnel Dorsale roundabout).

## Proposal

**1. Runs.** Before placing lanes, chain the edges into runs: consecutive edges of the same OSM way
and direction (`osm_id`, `is_reverse`), joined end to start, with the same lane count and the same
`oneway`. A run is one road.

**2. One placement per run.**
- Pairing (`_paired_gaps`) is decided for the run from its whole length, not per piece, so a 4 m
  piece can't disagree with its 10 m neighbour.
- Each lane is offset from the run's **merged** line once (`offset_curve` over the whole run), then
  cut back into the pieces at the nodes (the lane point nearest each node). Across a bend the lane is
  one continuous curve: no wedge, no jump. `placement` applies to the run too (its first tagged
  value).

**3. A continuing lane isn't cut.** In `_build_lane_connectors`, where lane k of a piece continues
into lane k of the next piece of the same run, neither end is trimmed at that node and no connector
is made: the road passes through the junction intact. Only lanes that end or turn there are cut.

**4. A minimum lane length.** Trimming never leaves less than `max(2 m, 50 %)` of a lane.

Connectors then appear only where lanes really change: turns, lane-count changes, one-way ↔
two-way transitions, forks and merges.

**Unchanged:** link ids, lane ids and numbers, movements and their lane ranges; `lane.geom` changes
shape only. Lane routing, lanestyle and the meso / micro networks read the same tables.

## Result on Monaco

| | before | after |
|---|---|---|
| lane continuations along one road needing a connector or leaving a gap | 678 | 10 (9 at junctions, 1 offset) |
| lanes shorter than 2 m | 288 | 87 (53 on links under 2 m, 32 on 2-4 m links, 2 at the 2 m floor) |
| connectors | 2,866 | 2,144 |

One more rule turned out necessary: a lane that goes on **and** turns (lane 2 straight on, a right
turn from it too) was still cut at its end for the turn's S-curve. A kept end stays put; the turn's
connector leaves from it.

## Checks (Monaco)

- Tests: a way in two pieces around a bend gives one continuous lane (no gap at the node); a
  4 m piece inherits its neighbour's pairing; a lane continuing through a junction keeps its full
  length and gets no connector; a 4 m piece keeps at least 2 m of lane.
- Re-count the table above (target: the 577 and the 55 and the 46 near zero), lanes under 2 m
  (288 → near zero), and the connector count (2,866: fewer, only real changes).
- Screenshots on the previews page (port 8090): the Tunnel Dorsale roundabout, a bend, a main road
  through a junction.
