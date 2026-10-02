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

## Driving only (Kaveh, 2026-10-01)

Lane ends and connectors are built for the driving mode only. For a footway OSM has no turning path, only shared nodes: its lane runs to its node
and meets the next one there (the walking `movement` table stays, for routing). Before, Monaco's walking schema had 11,310 connectors,
six times the driving ones, every one of them invented by the same code that trims a lane where it overlaps another.

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


## Widths and tight turns (Kaveh, 2026-10-01: "a service road is too wide", the U-turn and the dead-end blob)

- **Default lane width by class** (only where OSM has no `width:lanes`): `service` 2.5 m, `residential` / `living_street` / `unclassified`
  3.0 m, the rest 3.25 m (`_LANE_W_BY_CLASS`). It is now written into `lane.width` (not NULL), so lane placement, connectors, crossings
  and lanestyle all use one number. A bike or walk lane without a width stays NULL (lanestyle defaults it by use).
- **A connector no wider than its curve** (`_fit_width`): at most 1.8 × the tightest radius of its Bézier, never under 60 % of the lane width.
  A 90° turn in 3-5 m (a right turn off a service road, a U-turn) cannot carry a full lane; before, they folded over each other into a blob.
- Open: a dead end where three connectors leave one point still draws as a bulge (Monaco, `156780348#1f`); a half-circle curve for U-turns
  is not built.

## A lane no movement leaves, and a road without a lane count (Kaveh, 2026-10-02: `167625718#2f` lanes 2 and 3 "don't have any way out", the tunnel `80378487#1f` "has 3 lanes, not 2")

- **Continuation movements** (`_continuation_movements`, the root fix; first built as connectors only, moved to the movement table when Kaveh asked "I thought you fixed it in duckOSM's root"). Where a link has exactly
  one way on (U-turns aside) and OSM does not mark its lanes (`turn:lanes`), a car lane its movement does not cover merges into the last car lane of the next link (type `merge`), and a bike lane goes on into the
  next link's bike lane (type `thru`, `allowed_uses` bike): one movement row each (`<mvmt_id>-<lane>`, `<mvmt_id>-b<lane>`), same node and link pair. The lane connectors, the lane graph and every reader follow from the rows.
  Monaco: 18 rows (3,975 movements). Lanes that already meet (under 0.3 m apart) need no connector.
- **Lanes from the road it continues** (`_inherit_lanes`): a one-way edge whose way has no `lanes` / `turn:lanes` / `width:lanes` tag takes the lane count of the road it plainly
  continues (the node between them joins only the two, both one-way, one class; a road changes its name at a tunnel) when that has more, else of the one it leads into;
  repeated, so a way cut into pieces inherits piece by piece. Monaco: 1 edge (the tunnel). It is an inference, not OSM; `driving` only; log line `... take the lane count of the road they continue`.

A bike lane's connector (Kaveh, 2026-10-02: `919814602536534482_3>3913596625020233819_3`): its width was the car lane default (3.25 m) while the bike lane is drawn 1.5 m, and it had no bike colour. A connector now takes
`_DEFAULT_BIKE_W` (1.5 m) for a bike lane without a width (lanestyle draws the same), and lanestyle colours a connector of a bike lane as a bike lane.
