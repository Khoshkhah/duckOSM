# Lanes at junctions: netconvert and the lane rules

`duckosm gmns` writes, for each legal turn of a junction, which lanes it starts from and which lanes it ends in (the lane
ranges of `movement`, and `lane_connector`). Two engines can decide this:

| | **netconvert** | **the lane rules** |
|---|---|---|
| Code | SUMO's netconvert (C++): duckOSM writes the network to SUMO and reads the connections back | `rust/duckosm-export/src/gmns_lane_rules.rs`, ported from SimReady's lane rules |
| Needs | a SUMO install | nothing |
| Option | `--lane-source netconvert` | `--lane-source rules` |
| Default | no | yes |

Neither engine decides **which turns are legal**. GMNS decides that before, in `movement`, from the OSM turn restrictions.
The engines only decide **which lanes** serve each legal turn.

## Data and inference

OSM gives the number of lanes of a road, and sometimes the turn arrows painted on each lane. OSM has no tag named "arrow":
the arrows are in the tag `turn:lanes`, one value per lane from left to right, separated by `|` (for example
`turn:lanes=left|through|through;right`). OSM does not say which lane
goes to which lane at a junction. So for a lane with no arrows, both engines **infer** the lanes by rules. On Monaco, about
1 % of the lane pairs start on a lane with arrows; the other 99 % are inferred. The difference between the engines is
whose rules they are.

## The lane rules

The rules have SimReady's numbers (its page `docs/3-modules/connectors.md`):

| Rule | What it says |
|---|---|
| 1. Turn kind | GMNS's type of the movement: `left`, `right`, `uturn` (30° or more); a through, merge or diverge movement is `straight`, `slight_left` or `slight_right` by its angle (15° and 45°) |
| 2. Which lanes serve a turn | with `turn:lanes` arrows: the lanes whose arrows name the turn. Without arrows: by position. A left turn uses the leftmost lane, a right turn the rightmost lane, straight on uses all lanes, a U-turn the lane next to the median |
| 3. How lanes pair | a left turn pairs from the left side, a right turn from the right side. Straight on pairs side by side, and when the numbers differ, the extra lanes fan into the nearest lane: no lane ends with no way on |
| 4. Forks | at a diverge (one road in, two or more out) and a merge (two roads in, one out), the lanes are shared in their order from the left. At a merge, the more important road keeps its lanes |
| 5. Roundabouts | on the ring the lanes pair side by side; an entry and an exit pair from the outermost lane |
| 20. Bike lanes | a bike lane goes on to the bike lane of the road ahead, from the kerb side; not on a roundabout |
| 21. U-turn arrows | lanes with arrows but no `reverse` arrow: the U-turn takes the lane next to the median |
| 24. Fork arrows | at a fork (one road splits), a branch that the `turn:lanes` arrows leave with no lane takes the nearest lane that does not name the other side |
| Bus lanes | a bus lane pairs with a bus lane ahead, else with the kerb-side lane |

The rules run node by node on the GMNS links, which are the unmerged edges (`link_id = edge_id`). They do not use the
chains.

## Where the two differ

On Monaco (the same source db, the driving network), 97.5 % of netconvert's lane pairs are also the rules' pairs, and
3,503 of 3,629 turns (96.5 %) have exactly the same lanes. The other 126 turns differ in these ways:

| Turns | Case | netconvert | the lane rules |
|---|---|---|---|
| 22 | a merge of two roads into a two-lane road | each road's lanes go on side by side, without regard to the other road | the lanes are shared: the left road takes the left lanes, the right road the right lanes (rule 4) |
| 20 | two lanes go on into one lane | one lane goes on; the other lane has no way on | both lanes go on into the one lane (rule 3, fan) |
| 15 | a diverge into a two-lane branch | only the right lane goes on, into both lanes of the branch | both lanes go on, side by side (rule 4) |
| 12 | a right turn from two lanes into two lanes | the lanes go on side by side | both lanes go into the right lane ahead. Here netconvert's answer looks better: a rule to look at |
| 7 | a right turn from one lane into two lanes | the lane feeds both lanes | the lane goes into the right lane only (rule 3) |
| 50 | other small cases | | |

**Where OSM can judge** (the lanes with `turn:lanes` arrows): netconvert sends 13 of 35 lane pairs against their arrow, for example a lane
marked `right` that goes straight on. The rules follow every arrow (6 of their pairs are at forks, where a `left` arrow
leads into the left branch).

| Measure on Monaco | netconvert | the lane rules |
|---|---|---|
| time | 3.9 s | 1.2 s |
| lane pairs against their arrow | 13 of 35 | 0 of 29 |
| lanes with no way on / no way in (`lane_check`) | 14 / 0 | 5 / 4 |
| a SUMO network built from the result | builds | builds |
| path through the junction (`lane_connector`) | SUMO's internal lanes | a simple curve between the lane ends; the junction shape is Level 4 work |

The design and the full comparison: `docs/design/gmns_lane_rules.md`.
