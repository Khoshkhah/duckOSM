# GMNS: a road and its footpaths in one frame

Status: **the moves are OFF by default (Kaveh, 2026-10-01: they changed OSM's geometry and disconnected footways that meet at a node; `--walk-frame` turns them on, with `_walk_join` repairing the joints).** **step 1 and step 3 built; step 2 dropped** (Kaveh, 2026-10-01: clearance 0; "if there is no sidewalk or crosswalk, you can't add it there": an untagged footway is a real footpath wherever OSM maps it, never moved to a road); step 3 built (a footpath's two links are one strip, 2 m, centred on its line: `is_foot` in `_build_lane_curb`). Branch: `gmns-values`.

## The problem

In `gmns_walking` every footpath lane is placed from the footpath's *own* OSM line, and every road lane
from the road's own line. Two independent frames, so where a footpath runs along a road nothing keeps it
off the carriageway. Monaco (driving + walking, ground level, links that share no node):

| overlapping pair | pairs |
|---|---|
| footpath over road | 1,269 |
| footpath over footpath | 1,746 |
| road over road | 46 |

For the footpath-over-road pairs the footpath's centre line is a median 1.1 m (max 3.1 m) from the road
link's line: about 2 m of its 2 m strip lies on the carriageway. lanestyle can only hide this (it cuts
the lines of overlapping same-level links); the geometry is wrong at the source.

## The idea: the footpath inherits its road's frame

A sidewalk is a child of the road it runs along. GMNS already says so: `link.parent_link_id`
("for a sidewalk, this is the adjacent road"), set by `_sidewalk_parents` for ways tagged
`footway=sidewalk` (1,202 of Monaco's 6,922 walking footways). The child does not keep its own
position: it takes it from the parent's cross-section.

- **Frame.** The parent link's line, with lateral offsets in metres, left +. The parent's lanes already sit
  in it (`offsets()` in `_build_lane_curb`); its kerb-side edge is the outer edge of its last lane on that
  side: `edge = offs[last] - side * w_last / 2`.
- **Child lane.** Centre at `edge + side * (clearance + w_sw / 2)` from the parent's line (clearance 0 by
  default, a setting), as an offset curve of the *parent's* line, cut to the stretch the sidewalk covers
  (its end points projected onto the parent). So the sidewalk is parallel to its road and outside its
  lanes by construction, however the OSM way wanders.
- **Kerb side.** The side `_sidewalk_parents` already chose (the sidewalk's right with right-hand
  traffic, mirrored for `--drive-side left`).
- **Same-road pairs.** A two-way road is two links with the same line; a sidewalk on each side inherits from
  the link it is on the kerb side of, so the two sidewalks land on opposite sides.
- **No parent, no change.** A footpath with no parent (a path through a park, steps, a crossing) is placed
  as now.

## Steps

1. **Tagged sidewalks** (`footway=sidewalk`, parent exists): inherit as above. Test on a synthetic road
   with a sidewalk drawn on its edge and drawn 1 m inside it: both land clear of the lanes, `length_m` and
   `edge_id`/`link_id` unchanged (ids are final before geometry, as for `_fix_split_lengths`).
2. ~~Untagged footways along a road~~ **dropped (Kaveh, 2026-10-01).** It moved a real footpath to the kerb of a road that has no
   sidewalk. A footway without `footway=sidewalk` stays where OSM maps it.
3. **Footpath pairs.** Check first how a two-way footpath's two links are offset (the rule is the road's
   "two-way: each direction on its travel side"); if both land on the same strip, place the pair as one
   strip. Measure the 1,746 walk-over-walk pairs by cause before choosing a rule.

Each step reports how many overlaps remain on Monaco (the scan used above), and must not raise the count
of any other kind. Monaco only.

## Plan A: a sidewalk still on a road is pushed to the kerb (Kaveh, 2026-10-01)

Step 1 only frames a sidewalk on the kerb side of a road it has a parent for; 299 of Monaco's 1,330 sidewalk links (22 %) were left lying
inside a road surface and, with the "only a crosswalk may lie on a road" rule of lanestyle, vanished: one side of a street lost. `_walk_kerb`
moves each vertex of a mapped sidewalk (`footway=sidewalk`) that lies on a driving lane running along it out across the road, along the
sidewalk's normal, to the nearer kerb, half its width beyond it (+ `--walk-clearance`). A vertex on a lane that crosses the sidewalk (a side
road's mouth) is left alone: that is a crossing. Only mapped sidewalks move. Monaco: 740 sidewalk lanes pushed; links removed by the
lanestyle cut 299 -> 255, 169 of them pieces under 10 m across a side road's mouth.

## Not in this note

- `link.lanes`, lane numbers, movements and connectors: untouched. Only a footpath lane's `geom`
  (and the `lane` width where it is NULL) can change.
- Crossings where a footpath meets a road at a node: the lane ends stay at their nodes.
- lanestyle keeps its same-level overlap cut as a safety net; it hides nothing once the data is right.

## Result on Monaco (ground level, links that share no node, m² of overlap)

| | footpath over road | footpath over footpath |
|---|---|---|
| before | 34,195 (1,269 pairs) | 21,554 (1,746) |
| step 1 | 26,152 | 18,204 |
| steps 1 + 2 | 23,939 | 17,880 |

Between a sidewalk and **its own road** the overlap fell from about 14,000 to 324 m² (19 pairs). What is left is
mostly a footpath passing the mouth of a side street (it overlaps that road, which ends on a node of the main road:
a crosswalk, right as it is), crossings, and footpaths with no road along them within reach (the log says how many
lanes were left as they were, and why). A road without a sidewalk is not in the walking network; it is found in
`gmns_driving`, and `parent_link_id` stays empty then (the key would point out of its table).

## Open questions

- None open. (Step 3 was: a two-way footpath's two links each sat 1.625 m to their own right, two strips 3.25 m apart
  with a gap between them; now both lie on the footpath's own line, 2 m wide by default.)
