# The branches of a fork carry their own turn letter

**Status:** implemented 2026-10-04 (Kaveh: the arrow of lane `6602176641083707803_1` in Monaco "must have both turn left and straight"; "the shape of the arrow must come from the GMNS data").

## The problem

A fork (`diverge`, `_fork_types`) is a node one link arrives at and 2 or more ways leave, all within 45° of straight on. The angle typing (`thru` under 30°) made every branch of such a fork `thru`, so every `diverge` movement had `mvmt_code` `..T`. Avenue Prince Pierre in Monaco (`503813539#1f`)
splits into `503475651#1f`, a branch that bends to the left, and `503475647#1f`, which goes straight on, and both movements read `WBT`. A map that draws the lane's arrow from the movements could only say "straight" (and the design had no arrow for a fork at all).

## The rule

`type` stays `diverge`. The turn letter of `mvmt_code` is told apart by place, as `_turn_side` does for the lanes: at a fork the **straightest** branch (the smallest absolute angle) keeps `T`; another branch whose angle differs from it by **8 degrees or more** is `L` if it is to the left of it (the angle is positive to the left) and `R` if it is to the right; a branch within 8 degrees of the straightest one is `T`.
The angle of a branch is the heading of the outbound link over its first 15 m against the heading of the inbound link over its last 15 m. A shorter window or the first segment alone is wrong here: `503475651#1f` starts 6° to the left and bends on to due west (+11° over its 12 m), while `503475647#1f` runs straight (−10.5°); the first segment alone made the left branch look straight and the straight one right.
Two branches that run side by side stay `T`. A fork with all branches `T` is unchanged.

Monaco: the fork above reads `WBT` for `503475647#1f` and `WBL` for `503475651#1f`; see the commit for the counts. Merges (`merge`) are not changed.

## What does not change

`type`, the lane ranges of the movements, the lanes and connectors, `edge_id`s. Only the letter of `mvmt_code` of a `diverge` movement changes, as the GMNS pattern allows (R, L, T).

## The same angle for the lanes (2026-10-04)

The lane assignment (`_assign_lanes`) told which exit is straight, left or right of it from the same first-segment angle, and matched the OSM lane tags (`turn:lanes`) to the exits by that place. For `503813539#1f` (lane 1 `left`, lane 2 `through`) it called `503475651#1f` the straight exit and `503475647#1f` a right turn. No exit was `left`, so the `left` lane was added to the straight exit, and the exit left over found no lane tagged `right` and fell back to every lane: both lanes into both branches.
Where two or more exits go ahead (within 45°), their angle is now `_window_angle` (15 m on each side) for the lane assignment, and `_fork_letters` uses the same function. `type` (`thru` under 30°, `left`, `right`) is unchanged.
