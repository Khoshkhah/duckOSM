# duckosm.sim

Traffic simulation. The id of each SUMO edge and MATSim link is the duckOSM `edge_id`. pip extra: `duckosm[sim]`.

| Call | Returns | Today |
|---|---|---|
| `to_sumo(net, out_dir, mode=None, net_name="network", run_netconvert=False, connections=True, config=None)` | the SUMO files of one mode, as a dict of paths: `.nod.xml` (with the traffic lights and stop signs), `.edg.xml`, `.con.xml`, `.add.xml` (the bus stops) and `.netccfg` (the netconvert config; run `netconvert -c` on it). With `run_netconvert=True`, also `.net.xml` and `.tll.xml` (the signal programs). The dict also has `n_traffic_lights`, `n_stop_signs` and `n_bus_stops`. [SUMO](../../exports/sumo.md) | yes |
| `to_matsim(net, out_path, mode=None, crs=None, gzip=True)` | the MATSim `network.xml`; `mode` can be a list | yes |
| `to_matsim_lanes(gmns_db, out_dir=".", mode="driving", signals=True, cycle_s=90)` | MATSim lanes and signals, from a GMNS database | yes |
| `sumo_net(path)` | the SUMO network (a `.net.xml`, or the dict of `to_sumo`) read with sumolib, with the internal edges: a `sumolib.net.Net` | yes |
| `edge_map(path)` | a DataFrame of the id of each SUMO edge (`.net.xml`, `.edg.xml`) or MATSim link (`network.xml`, `.xml.gz`) and its `edge_id`. A netconvert id `123#1` gives 123; `123+456` gives a row for each edge; SUMO's internal edges are not in it. With `geometry.remove=true` (the default of `to_sumo` is `false`), netconvert joins edges and keeps only the id of the first edge, without `+`: the other edges of the join are not in the table (on Monaco driving, 1615 of 2840 edges) | yes |

```python
files = net.driving.to_sumo("out/sumo")            # = duckosm.sim.to_sumo(net.driving, "out/sumo")
sn = duckosm.sim.sumo_net(files)                   # sumolib.net.Net
ids = duckosm.sim.edge_map(files)                  # SUMO id -> edge_id: join the output of SUMO on "id"
```

Notebook: [12. Traffic simulation](../../notebooks/12_sim.md). Guide:
[duckOSM for traffic simulation users](../../compare/for-simulation-users.md).

Pages: [SUMO](../../exports/sumo.md), [MATSim](../../exports/matsim.md).
