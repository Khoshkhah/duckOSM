# Edges and chains

duckOSM has two units for a road: the **edge** and the **chain**. Each level of the outputs uses one of them as its unit,
for a reason.

## The two units

| Unit | What it is | Id |
|---|---|---|
| **Edge** | one direction of a road between two cut points. duckOSM cuts a road at each junction, and also where a footway or a cycleway crosses it, so that the walking and cycling networks meet the road there | `edge_id`, a stable hash ([Stable edge ids](edge-ids.md)) |
| **Chain** | the edges of one road between two real junctions, joined again over the footway crossings. A chain stops at a traffic light, a stop sign, a turn rule, or where the tags or the lanes change. Driving only | `chain_id`: the `edge_id` for a chain of one edge, else a hash of its edges in order. A chain lists its edges (`edge_ids`, GMNS: `link_ids`) |

**The edge is the id everywhere.** A chain is a coarser view of the same edges, never a replacement: each output that uses
chains keeps the list of their edges, so that results join on `edge_id`.

## The unit of each level

| Level | Unit | The other unit | Why |
|---|---|---|---|
| 2. Routable network | edge (`<mode>.edges`) | chains in `driving.chains` | routing and turn restrictions need every node, the footway crossings too: there the modes meet |
| 3. GMNS network | link = edge (`link_id = edge_id`) | chains in `gmns_driving.chains` | GMNS is a network to move through: routing, assignment, simulation for all modes. A crossing is a real node, where the walking and cycling links join the road. The movements (turns) are stored node by node, and the lane rules work node by node ([Lanes at junctions](lane-rules.md)) |
| 4. HD map (the HD map database of SimReady) | chain: one carriageway for each chain of `gmns_driving.chains`, with the same `chain_id` | the chain's `link_ids` | the HD map describes the road surface. A footway crossing is a crosswalk painted on the carriageway, not a break in it, so the lane lines, widths and heights run on along the chain. The `link_ids` lead to the GMNS lanes and turns of each part |

## Simulation exports

| Export | Default | With `--merge` |
|---|---|---|
| SUMO (from Level 2 or GMNS) | one SUMO edge for each edge | one SUMO edge for each chain; its `<param key="edge_ids">` lists the edges |
| MATSim `network.xml` and `matsim-lanes` | one link for each edge | one link for each chain; its attribute `edge_ids` lists the edges |

`duckosm.sim.edge_map` reads both lists, so the results of a merged network still join on `edge_id`.

## Rules

- A level never loses its edges: the lower level is the source, and the chain lists its edges.
- Chains are driving only: walking and cycling keep their edges.
- A chain is cut at each node inside it where another chain starts or ends. So the two directions of a two-way road have the same
  chains, in `driving.chains` and in `gmns_driving.chains`.
- The chain table of GMNS comes from the Level 2 chains, cut where the GMNS lanes change: a merged SUMO edge has the lanes of
  its last link, so each link of a chain must have the same lanes.
- A build by tiles makes the chain table once, after the tiles are joined, so no chain stops at a tile border.
