# GMNS lanes: one lane profile, connections from SUMO (design, 2026-10-10)

Status: **proposal**, nothing built. Replaces the lane rules spread over the build and `gmns.py`.

## Why

A road's lanes are decided today in five places, and later steps undo earlier ones:

| where | what it does to the lanes |
|---|---|
| `processors/road_filter.py` | `lanes_fwd` / `lanes_bwd` from `lanes`, `lanes:forward/backward`, the contraflow bus lane |
| `gmns._inherit_lanes` | an untagged one-way edge copies the count of the road it continues |
| `gmns._build_lane_curb` (228 lines) | adds a bike lane (right, or left as lane -1), a bus-only lane, widths, placement, geometry |
| `gmns._assign_lanes` + `_fork_lanes` / `_merge_lanes` / `_default_lanes` | guesses which lane turns into which (angle thresholds 45 and 8 degrees, names, classes) |
| `gmns._fork_branch_lanes` | **deletes** lanes the guesses left without a way in (19 in Monaco until the roundabout fix) |

Plus 17 `or 1`, 16 `COALESCE` defaults and 9 `except` blocks: wrong data comes out as "1 lane" or "3.25 m", not as a report.
Each Monaco case found in Street View (2026-10-10) came from a different one of these places.

## a) One lane profile per directed edge, in the duckOSM build

A table **`<mode>.lane_profile`**, written once by the build (after `OsmOverrides`, which it reads), read by everything else
(GMNS, the SUMO export, roadstyle / lanestyle). Nothing after it adds or removes a lane.

| column | meaning |
|---|---|
| `edge_id` | the directed edge |
| `lane_num` | GMNS numbering: 1 = the left-most through lane, 1..n the motor lanes; a lane left of lane 1 is -1, -2 (spec); a lane right of them n+1 |
| `use` | `auto`, `bus`, `bike`, `bus,bike` |
| `width_m` | the lane's width |
| `turn` | its `turn:lanes` entry, or NULL |
| `source` | the tag (or rule) that decided it: `lanes`, `lanes:forward`, `bus:lanes`, `cycleway:left`, `override`, `inherited`, `default` ... |

`source = 'default'` is how a missing tag shows: a class default stays visible and countable, never silent.

**The rules, one function, a table of tag cases, a test per row** (first match wins; draft from what Monaco needed):

| question | rule |
|---|---|
| motor lanes this way | `lanes:forward` / `lanes:backward`; else a one-way: `lanes` minus the bus lanes the other way (`lanes:backward`, else 1 when `oneway:bus/psv=no` or `busway:*=opposite_lane`); a two-way: `lanes` split (forward the larger half); none tagged: inherited from the road it plainly continues, else the class default |
| which motor lane is a bus lane | `bus:lanes` / `psv:lanes` / `lanes:bus`, `busway:<side>=lane` (its side) |
| bike lanes (not in `lanes`, as OSM says) | `cycleway:right=lane`: right of the motor lanes; on a one-way, `cycleway:left=lane` with the flow: lane -1; on a two-way, `cycleway:left` is the other way's right; `share_busway`: the bus lane becomes `bus,bike`; `opposite_lane` / `oneway:bicycle=no`: a bike lane on the contraflow edge |
| widths | `width:lanes`, `cycleway:*:width`; else by use and class (`source = 'default'`) |
| turns | `turn:lanes(:forward/:backward)`, one entry per motor lane |
| overrides | `osm_overrides.yaml` (`lanes`, `lanes_forward/backward`, later `tags:`) before any rule |

## b) Which lane goes to which, and the path through the junction, from SUMO netconvert

As urbanstyle does today (`urbanstyle/sumo.py`):

1. `to_sumo` writes each edge's lanes from the profile, per lane: `<lane index allow width>` (SUMO counts from the right:
   `index = lanes - position`), keeps the edge ids and the legal turns of `edge_graph` (`.con.xml`).
2. netconvert decides the lane-to-lane connections, the junction shapes and the internal lanes (the paths through a junction).
3. GMNS reads the `.net.xml`:
   - `movement`: one row per legal turn as now; its lane ranges from netconvert's connections;
   - `lane_connector`: the internal lanes' shapes, so the connectors meet the lanes exactly;
   - `lane.geom`: SUMO's lane shapes (they stop at the junction shape); `lane.geom_full`: our own line to the node, for drawing
     without connectors.

Removed from `gmns.py`: `_assign_lanes` and its four rules, `_fork_branch_lanes`, the Bézier connectors and their cutting
(`_build_lane_connectors`). Kept: node / link tables, crossings, signals, the walking network.

netconvert becomes a requirement of a GMNS build with lanes (`duckosm[sumo]`): missing or failing, the build stops with the reason.
A legal turn netconvert gives no lane connection (it drops some U-turns: 37 in Monaco) stays a movement and is listed in a report.

## Checks (reported, never patched)

After a build: every motor lane has a way in and out (or is listed); each edge's motor lanes match its tags (or the rule that
overrode them is named); every default used is counted per kind. Printed and stored, not fixed silently.

## How it replaces the old code

Build Monaco both ways and compare per link (lane count, uses, widths) and per movement (lane ranges). Every difference is
listed with its OSM way, checked (Street View where needed), and only then does the new code replace the old.

## Decisions (2026-10-10)

1. **Bike lanes are not counted in `lanes`**, as OSM says, plus overrides where mappers counted them (Avenue Princesse Grace).
   Checked on SUMO's own OSM import of Monaco (netconvert `--osm-files`): it adds the bike lane on top too (one-way `lanes=2` +
   `cycleway:right=lane`: 2 car lanes + a bike lane; two-way `lanes=2` + `cycleway:left=lane`: 1 car lane each way + a bike lane).
   One difference: on a one-way way, SUMO reads `cycleway:left=lane` as a bike lane AGAINST the traffic (a separate edge); OSM
   (and Street View on Avenue Princesse Grace) has it with the traffic unless `oneway:bicycle=no` / `cycleway:left:oneway=-1`.
   The profile follows OSM; SUMO gets the profile's lanes, so its own reading never applies.
2. **Lane lines**: ours for `geom_full` (with the dual-carriageway shift), SUMO's at junctions. **SUMO's algorithm is fed only what
   its own OSM import would give it** (each edge's line, its lanes with their use and width, the legal turns): no lane shapes, no
   junction shapes, no lane-to-lane connections of ours. What it builds is read back as it is. A check compares, per edge, the
   lanes of this run with SUMO's own OSM import and lists every difference (each one a tag the profile reads differently, named).
3. **No joining of close nodes** (`junctions.join false`): GMNS nodes stay duckOSM nodes.

## Steps

1. The lane profile table and its rule tests (duckOSM build); Monaco: list every edge whose lanes differ from today's GMNS.
2. `to_sumo` writes the profile's lanes per lane; the comparison with SUMO's own OSM import.
3. GMNS reads movements' lane ranges, connectors and junction lane shapes from the `.net.xml`; Monaco old vs new, per link and
   per movement; the old rules removed after the check.
