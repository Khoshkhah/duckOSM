# A fork branch keeps the lanes the fork feeds

**Status:** implemented 2026-10-04 (Kaveh: "if something is an issue in the data we should fix its root", after four arrows stood on two lanes at a fork in Monaco).

## The problem

A road of 2 lanes splits into two one-way roads, each tagged `lanes=2` in OSM (Avenue Prince Pierre `503813539#1f` into `503475647#1f` and `503475651#1f`, Monaco, lon 7.4182 lat 43.7327). Four lanes were drawn side by side over the branches' first 12 m, but with the OSM lane tags (`left|through`) the lane assignment feeds only one lane of
each branch: lane 1 goes to the left branch, lane 2 to the straight one. The other two lanes had no movement leading into them and began, beside the fed lanes of the other branch, at the node.

## The rule

`_fork_branch_lanes`, after the lane assignment (`_assign_lanes`): a branch of a fork (the outbound link of a `diverge` movement) that has lanes no movement leads into **keeps only the lanes some movement leads into**, when
- the branch has at least two motor lanes, and every movement into it names its outbound lanes;
- it goes on (U-turns aside) into exactly one link, with at least as many lanes as the branch was drawn with: the lanes that were left off begin at that next link;
- the lanes without a movement are at the edge of the branch, not between fed ones.

The unfed lanes are deleted from `lane`; the lane numbers of the rest stay (lane 2 of 2 is the right lane); `link.lanes` is the number left; the movements out of the branch are cut to the kept lanes, the outbound lanes moving with them (a branch with lane 2 only goes on lane 2 into the next link). A movement left with no lane is deleted.

Monaco: 14 lanes of 14 fork branches. For the fork above, `503475647#1f` keeps lane 2 (into lane 2 of `503475647#2f`) and `503475651#1f` keeps lane 1.

## What does not change

Edges, `edge_id`s, `link_id`s, node ids, the lanes of every other link, the fork types and the letters of `mvmt_code`. The OSM tag `lanes=2` is not changed in the source; the GMNS link says how many lanes the first piece of the branch carries.
