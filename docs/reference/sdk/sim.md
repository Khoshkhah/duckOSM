# duckosm.sim

Traffic simulation. The id of each SUMO edge and MATSim link is the duckOSM `edge_id`. pip extra: `duckosm[sim]`.

| Call | Returns | Today |
|---|---|---|
| `to_sumo(net, out_dir, mode=None, net_name="network", run_netconvert=True, connections=True, config=None)` | the SUMO files of one mode (`.nod.xml`, `.edg.xml`, `.con.xml`; with SUMO's `netconvert` also `.net.xml`), as a dict of paths | yes |
| `to_matsim(net, out_path, mode=None, crs=None, gzip=True)` | the MATSim `network.xml`; `mode` can be a list | yes |
| `to_matsim_lanes(gmns_db, out_dir=".", mode="driving", signals=True, cycle_s=90)` | MATSim lanes and signals, from a GMNS database | yes |
| `sumo_net(path)` | the SUMO network read back with sumolib | later |
| `edge_map(path)` | the SUMO or MATSim id → `edge_id` (to join the results back) | later |

```python
net.driving.to_sumo("out/sumo")                    # = duckosm.sim.to_sumo(net.driving, "out/sumo")
```

Pages: [SUMO](../../exports/sumo.md), [MATSim](../../exports/matsim.md).
