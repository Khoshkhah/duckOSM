# GMNS lanes that connect: lane ends and lane connectors

**Status:** asked for by Kaveh 2026-09-30 ("fix it"), built the same day on branch
`paired-carriageways` (`_build_lane_connectors`). Checked on Monaco only.

## Problem

`lane.geom` is the link's centre line shifted sideways. So every lane ends level with its link's end
node, placed from the road, never joined to the lane it continues into. The only turn path, GMNS's
`movement.geometry`, is a curve between points on the two road centre lines. In Monaco, the gap
between a lane's end and the start of the lane it leads into (median / 90 %):

| movement | pairs | gap |
|---|---|---|
| thru | 2,462 | 0.08 / 0.83 m |
| left / right | 543 / 694 | 2.1 / 3.2 m |
| uturn | 234 | 3.9 / 4.4 m |
| merge / diverge | 193 / 152 | 0.5 / 1.9 m, 0.4 / 2.1 m |

Kaveh's reports:
- one-way ↔ two-way: Avenue de la Costa → Avenue d'Ostende (4569630555833613661 → 1400975858733323105)
  and Avenue de Monte-Carlo, where the lane jumps 1.7 m in the middle of the road;
- a fork: Boulevard du Larvotto → Bretelle Ostende (545218161373926197 → 779727687145842510), 1.6 m;
- at junctions, every lane runs into the middle and overlaps the others.

## How lane-level formats do it

SUMO (internal lanes), OpenDRIVE (connecting roads in a junction) and Lanelet2 (lanelets inside an
intersection) all stop lanes at the junction's edge, and give each lane-to-lane connection its own
path. GMNS has no table for that: `movement` has one road-level curve per movement.

## Proposal

After the movements and their lane ranges (`gmns_lane_movements.md`), in the local metre frame:

1. **Lane ends.** Each lane is shortened at each end, by the larger of:
   - **at a junction** (a node with 3 or more neighbours): the stretch where it lies inside the
     other links' lanes there (all but its own link and its reverse), plus 0.5 m;
   - **where it doesn't meet the lane it connects to** (a gap over 0.25 m): half of
     `max(3 m, 2.5 × gap)`, so the connector has room for a gentle S-curve.

   At most 40 % of the lane, at each end. A lane that meets its partner (a straight road going on)
   isn't shortened.
2. **Lane connectors.** For each lane pair of a movement (the k-th inbound lane into the k-th
   outbound lane), a cubic Bézier from the end of the inbound lane to the start of the outbound
   lane, with tangents along both lanes. They go into a new table, `gmns_<mode>.lane_connector`
   (`connector_id`, `mvmt_id`, `from_lane_id`, `to_lane_id`, `width`, `geom`), a documented duckOSM
   extension to GMNS. Pairs that already meet (under 0.3 m apart) get none.
3. **`movement.geometry`** stays the standard road-level curve.

**Readers:**
- lanestyle draws the connectors like lanes: same width, the inbound lane's level, no arrows, no
  lane lines. When a lane is clicked, it colours the connectors to the next lanes too.
- The micro network's connectors start from the new lane ends automatically.

## Result on Monaco

2,789 connectors. The gap from a lane's end through its connector to the next lane's start is now
0.00 m (median) for every movement type, at most 0.34 m (pairs under 0.3 m apart get no connector).
Before, it was 2-4 m for turns. `duckosm gmns` still takes about 4 s for Monaco.

lanestyle draws the connectors and, once they're there, ends lanes flat (round ends had filled the
gaps; they bulged past lane ends and showed as discs in translucent tunnels).

## Checks (Monaco)

- Tests:
  - a connected lane's end and its connector's start coincide;
  - a one-way → two-way continuation gets an S-shaped connector;
  - a straight continuation gets none;
  - a junction's lanes are shortened.
- Monaco: re-measure the gaps above (lane end → connector → next lane: 0), and screenshot the three
  reported places on the previews page.
