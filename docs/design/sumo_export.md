# SUMO export: design details

The user page is `docs/exports/sumo.md`. This file keeps the details that are too long for it.

## Lanes

In a driving network, the lanes of each edge come from `driving.lane_profile` (`gmns_lane_profile.md`). The export writes
one SUMO `<lane>` for each lane, with its vehicle classes and its width, numbered from the right as SUMO does.

## Turn arrows

SUMO's plain XML has no field for painted turn arrows (`turn:lanes`), so `duckosm sumo` does not write them. The GMNS export
uses them (`lane_check` in `docs/exports/gmns_tables.md`): where the arrows of a link fit its exits, read left to right by
order, netconvert's OSM import connects the lanes of that link by them. These lane connections go into the plain XML.

## Join close junctions

`junctions.join` is off by default. A crossroads mapped as two or three close nodes becomes one junction when it is on.

The problem: netconvert reads the turns (`.con.xml`) before it joins the nodes. It drops each turn onto an edge that the join
removes (the short edge between two joined nodes), and it gives no message. A road whose straight or left turn used that edge
loses the turn.

The procedure when joining is on:

1. Run netconvert once with `--junctions.join-output` to get the node groups that it joins.
2. Merge those nodes (`cluster_<ids>`, the netconvert name).
3. Write each legal path through the removed edges as one turn. A path never goes back through a node that it passed.
4. Run netconvert again without joining.

Turn restrictions stay: a path is legal only if each of its steps is in `edge_graph`.

Only the Python `to_sumo(config={"junctions.join": "true"})` does this. The Rust export refuses `junctions.join` without a
`.netccfg` file (`rust/duckosm-export/src/sumo.rs`: "not ported to Rust").
