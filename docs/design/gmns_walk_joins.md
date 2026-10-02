# GMNS: where a footway meets a road, the connector is data

**Status:** approved by Kaveh and built 2026-10-02 (`_build_walk_joins`). Branch `gmns-values`. Monaco only.

**Result (Monaco):** 542 joins, as many as lanestyle made; all 542 footway ends the same, 524 of the joins identical, 18 start from another road lane end at almost the same distance (within 0.1 m). `duckosm gmns` driving + walking: 44 s in all (walking is the slow mode).

## Problem

A road lane is placed beside its link's line; a footway lane lies on its own line. At a node both links share, the two lane ends are up to a lane
width apart. Today the gap is closed by **lanestyle** (`_join_footways`, 542 connectors in Monaco): a drawing tool inventing network objects.
Kaveh (2026-10-02): "lanestyle is a visualization tool, it isn't right to add connectors on its own." duckOSM only builds connectors for the
driving mode (`gmns_lane_connectors.md`: "Driving only"), so the data has no footway-to-road connector at all, and `gmns_walking` has no
`lane_connector` table.

## Proposal

After the walking lanes are placed (`_build_lane_curb`) and the walking movements exist, `_build_walk_joins(con, sch)` writes
`gmns_walking.lane_connector` (the same columns as the driving one: `connector_id`, `mvmt_id`, `from_lane_id`, `to_lane_id`, `width`, `geom`):

- **Where:** a node that a footway link and a road link share in the data (never between links that share no node: no link OSM does not map).
- **Between:** the end of each footway lane at that node and the end of the nearest road lane there, when the two ends are 0.3-6 m apart
  (lanestyle's rule today, so Monaco gives the same 542 pairs to compare against).
- **Shape:** a straight segment from the road lane's end to the footway's end. `from_lane_id` is the road lane (`<link>_<n>`, the same id in
  `gmns_driving` and `gmns_walking`), `to_lane_id` the footway lane. `width` is the footway lane's width. `mvmt_id` is the walking movement
  from the footway onto the road when there is one, else NULL.
- Nothing moves: no lane changes, no new link; `movement` is untouched. The footway still runs to its OSM node.

## What lanestyle changes

- `_join_footways` is deleted. lanestyle only reads.
- The multi-mode reader keeps a walking connector whose two lanes are both kept (today it drops every later-mode row of a link already seen, and a
  connector carries its leaving lane's link).
- A connector's modes stay "the modes both its lanes share" (walking), drawn as now (a footway connector, under the footway ranks).

## Checks (Monaco)

- Tests: a footway and a road one lane-width apart at a shared node get one connector; two links with no shared node get none; a gap under 0.3 m
  or over 6 m gets none.
- Monaco: the row count equals lanestyle's 542 before the move; the lanestyle map renders the same pixels at the Boulevard Rainier III spot and two
  other joins (`renders/footway_end/`).
- `duckosm gmns` build time stays near 4 s.

## Not in this note

- The U-turn at a road end (`4030367642102153402_1>6871880265432049446_1`, Boulevard Rainier III): its connector is a 4.7 m tight Bézier, 1.8 m wide;
  duckOSM's open item "a half-circle curve for U-turns is not built". A separate note.
- Footway-to-footway joins: both lanes already end at the shared node, no gap.
