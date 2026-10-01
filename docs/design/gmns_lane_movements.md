# GMNS movements at lane level: a better calculation from more OSM data

**Status:** approved by Kaveh 2026-09-30 (with osm2gmns's separate lanes). Steps 1-6 built (branch
`paired-carriageways`), checked on Monaco. Improves GMNS's own calculation in
`_build_movement` (`gmns.py`); it doesn't replace it. Which turns exist stays as today: `edge_graph`
(OSM restrictions applied) plus GMNS's rule for immediate U-turns (kept where they're the only way on
or the only way in). This note is about **which lanes** feed and receive each turn. Checked on Monaco
only.

## Problem

A GMNS movement says which lanes a turn starts from (`start_ib_lane`..`end_ib_lane`) and which it
ends in (`start_ob_lane`..`end_ob_lane`). Today:

- **The inbound range** comes from `turn:lanes` only. Monaco has it on 14 driving ways (33 lanes),
  so 26 of 3,957 movements have an inbound range. Everywhere else it's NULL.
- **Multi-valued lanes are misread.** `through;slight_right` (3 lanes in Monaco) and
  `through;right` (1) count only as straight-on lanes (the `lt` CASE takes the first match), so the
  right turns that start from them get no inbound range.
- **The outbound range is never set** (always NULL).
- **The tools that read movements read NULL differently:**
  - lane routing (`lane_routing.py`) honours the inbound range but connects to every outbound lane;
  - the micro network (`gmns.py`, movement connectors) builds one connector from the first inbound
    lane (or lane 1) into outbound lane 1;
  - `gmns-map` and lanestyle read NULL as "every lane".

  So a left turn can start from the right lane, and the same movement means different things in each
  tool.

418 of Monaco's 3,092 links have more than one lane. That's where this matters; on a 1-lane link
every range is just lane 1.

## Proposal, in small steps (each its own commit and check)

Lane numbers as today: lane 1 is the leftmost in the direction of travel, n the rightmost.

### Step 1: read `turn:lanes` fully

Split each lane's value on `;` and map every part to the movement types it allows:

| `turn:lanes` part | movement type |
|---|---|
| `through` | thru |
| `left`, `slight_left`, `sharp_left` | left |
| `right`, `slight_right`, `sharp_right` | right |
| `reverse` | uturn |
| `merge_to_left`, `merge_to_right`, `none`, empty | thru |

So `through;slight_right` feeds both the thru and the right movement. Movement types still come from
the angle at the junction, as today. A `slight_right` lane whose turn measures under 30° (typed
`thru`) also feeds that thru movement, because `through` and `slight_*` both cover it.

**Fixed after Kaveh's report (Boulevard Charles III, 6594925326949888649_2 / 7306218974710360300_2):**
- `turn:lanes` applies only where its way ends ("to the junction", OSM wiki Key:turn). duckOSM splits
  a way into pieces at other junctions, and every piece carried the arrows, so a right-turn lane had
  no way on where no right turn exists. Now, along the way (a movement into the next piece of the same
  OSM way, same direction), every lane continues lane by lane.
- Arrows match exits by their place, not by the angle type (`_turn_side`): the straightest exit
  within 45° takes `through`, exits left of it `left`, right of it `right`. A slight fork typed `thru`
  by its angle still takes the right-turn lane.
- Neither osm2gmns (its `turn:lanes` module is an empty stub) nor SUMO's docs say how they do this.

### Steps 2 and 3: default lanes where `turn:lanes` is missing (osm2gmns's rules)

Separate lanes per turn, as osm2gmns does (`autoconintd.py`): the outbound links of an inbound link
are sorted left to right by angle.

| case | lanes |
|---|---|
| 1 inbound lane | every turn starts from it; the leftmost outbound link is entered at its lane 1, the others at their rightmost lane |
| 1 outbound link | `min(n, m)` lanes from the left, in order |
| 2 outbound links | the right one: the rightmost inbound lane into its rightmost lane; the left one: the remaining lanes, from lane 1 |
| 3 or more | the leftmost: lane 1 → lane 1; the rightmost: lane n → its rightmost lane; the ones in between share the middle lanes |

On a 3-lane approach to a 4-way junction: left from lane 1, straight on from lane 2, right from
lane 3. No lane serves two turns. `turn:lanes`, where tagged, overrides this (step 1).

**How a row says "lane 1 → 1, lane 2 → 2"** (Kaveh asked how others do it; checked 2026-09-30):

| | lane pairing |
|---|---|
| GMNS spec (`movement`) | one range per side: `start_*_lane` innermost, `end_*_lane` outermost; a blank end means a single lane; it says nothing about pairing |
| osm2gmns 0.7.6 | one row per movement; the inbound and outbound ranges always have the **same length** and are read **in order**, the k-th inbound lane into the k-th outbound lane (`createMicroNetForConnector`). Rows whose ranges differ are trimmed to the shorter one, with a warning (`validateUserInputMovements`) |
| SUMO, OpenDRIVE, OSM `connectivity` | one record per lane pair |

**So we follow osm2gmns:** one row per movement, ranges of equal length, read in order. Where the
counts differ, `min(n, m)` lanes connect. No extra rows are needed for lane 1 → 1, lane 2 → 2.

**Decided: separate lanes, as osm2gmns** (Kaveh's OK 2026-09-30; he asked for a suggestion after
the comparison). The alternative was shared lanes (straight on from every lane, so lane 1 is left +
straight on): closer to many real roads without arrows, but our own rule, and it would make our GMNS
read differently from other GMNS data.

**U-turns keep their own lane, as osm2gmns (Kaveh, 2026-09-30, option A).** On 3 multi-lane approaches
in Monaco a U-turn into the other carriageway takes lane 1 to itself (e.g. Boulevard du Larvotto,
5405468474044964645: lane 1 U-turn only, straight on from lane 2). Option B, U-turns left out of the
split and sharing lane 1, is the fallback if a case shows A is wrong.

### Step 4: the tools read the ranges the same way

- **lane routing:** honour the outbound range too, pairing in order.
- **micro network:** one connector per inbound lane in the range, into the matching outbound lane.
- **`gmns-map` and lanestyle** read both ranges but connect every lane in one range to every lane in
  the other; they too pair in order.

### Step 5: lane use from `bus:lanes` and `access:lanes`

Monaco has 6 ways with each. `bus:lanes=designated` makes a bus lane, as `psv:lanes` already does.
`access:lanes=no` together with a bus or psv designation makes the lane closed to cars.

**Built (step 5):** Monaco has 8 bus lanes now, all on Boulevard Princesse Charlotte. Its 3-lane
road narrows to 2 where the bus lane ends: straight on pairs lanes 1-2 into 1-2 (osm2gmns keeps the
leftmost), so a bus changes into lane 2 before the end (lane routing has lane changes). Also fixed in
step 1: a lane left empty in `turn:lanes` (stored NULL) counts as straight on.

### Step 6: lane geometry from `placement`

Monaco has 13 ways with it. `placement` says where the OSM line lies across the lanes
(`left_of:1`, `middle_of:2`, `right_of:2`, `transition`). Today the line is taken as the carriageway's
centre (one-way) or the centre line (two-way). With `placement`, the lanes are offset from where the
line really is. This is a geometry step, next to
[gmns_paired_carriageways.md](gmns_paired_carriageways.md).

**Built (step 6):** `left_of:N` / `middle_of:N` / `right_of:N` give the line's position across the
lanes, and each lane is offset from there (`_placement`). Placement is data, so it wins over the
paired-carriageway estimate. `transition` and unusable values keep the default. On a two-way way it
applies only in right-hand traffic. Monaco: 14 ways, mostly 1-lane one-way ramps (`left_of:1` /
`right_of:1`, 6 `transition`); Boulevard Charles III's `placement:forward=left_of:1` matches the
two-way default.

### Step 7 (Kaveh, 2026-09-30): merges and forks

- **Lanes at a merge:** osm2gmns's merge rule (`autoconm.py`, ported as `_merge_lanes`), for a node
  with one outbound link. The inbound links are sorted left to right; the leftmost one's rightmost
  lanes go into the outbound link's leftmost lanes, every other link's leftmost lanes into its
  rightmost lanes. Before, every joining road went into lane 1.
- **Types `merge` and `diverge`** (GMNS allows them; osm2gmns doesn't set them), in `_fork_types`:
  - `diverge` at a fork: one link arrives, and its 2+ ways on (U-turns aside) are all within 45° of
    straight on;
  - `merge` into a node one link leaves, its 2+ inbound links all joining within 45°.

  The 45° bound keeps an ordinary junction a junction. `mvmt_code` keeps the angle's letter (the
  GMNS pattern allows only R/L/T); lane routing costs them like straight on.
- **Monaco:** 143 diverge and 133 merge movements. Lanes no movement leads into: 200 → 140.

### Not now: `connectivity` relations

`type=connectivity` relations give explicit lane → lane mappings and would override steps 2 and 3
where present. Monaco has none, so this can't be checked here. It stays for when an area has them.

## Result on Monaco (steps 1-3)

All 3,957 movements now have both lane ranges (before: 26 inbound, 0 outbound), 4,210 lane-to-lane
connections paired in order. 200 of 3,520 lanes have no movement leading in; 183 of them are on a road
another lane of which is entered, so they're reached by changing lane (lane routing has lane changes);
the other 17 are the 16 links with no way in at all (car park doors, the extract's edge, gates).

## Checks (Monaco only)

- Tests per step in `tests/test_gmns.py`:
  - a `through;right` lane feeds both movements;
  - a 3-lane approach without `turn:lanes`: left from lane 1, right from lane 3, thru from 1..3;
  - outbound defaults;
  - lane routing honouring the outbound range;
  - a bus lane from `bus:lanes`.
- Rebuild Monaco's GMNS db. Count the movements with inbound and outbound ranges set (now 26 and 0)
  and the lanes with no way in (now 16; the defaults must not add any). Show clicked lanes on the
  lanestyle previews page (port 8090): a left-turn lane connects only to the leftmost lane it turns
  into.
