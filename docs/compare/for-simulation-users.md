# duckOSM for traffic simulation users

This page is for users of SUMO (netconvert, osmWebWizard, sumolib) and MATSim (pt2matsim, matsim-tools). Each table
gives the call of your tool and the duckOSM call that does the same work. The module is `duckosm.sim`
([reference](../reference/sdk/sim.md)). The notebook [12. Traffic simulation](../notebooks/12_sim.md) runs each call
on Monaco.

```bash
pip install "duckosm[sim]"      # SUMO (netconvert, sumo) and sumolib
```

## SUMO

| Your tool | duckOSM |
|---|---|
| `netconvert --osm-files map.osm` | `net.driving.to_sumo("out/sumo")`: the plain XML of the duckOSM network (with the traffic lights, stop signs and bus stops) and a `.netccfg`, then `netconvert -c out/sumo/network.netccfg` |
| osmWebWizard (download and convert) | `duckosm.network("Monaco").driving.to_sumo("out/sumo", run_netconvert=True)` |
| `sumolib.net.readNet("map.net.xml")` | `duckosm.sim.sumo_net(files)` |
| `edgeData` output, by SUMO edge id | `duckosm.sim.edge_map(files)`: join on `id`, then on `edge_id` |
| fix a network by hand after netconvert | a rule in the fixes file ([Fix the OSM data](../guides/edit-osm-data.md)); it stays across OSM updates |

| | netconvert from OSM | duckOSM |
|---|---|---|
| Edge id | from the OSM way, changes when OSM changes | `edge_id`, the same on each rebuild |
| Turn restrictions | from OSM | from OSM, and your own in the fixes file |
| Lanes | from OSM, or a default | from OSM, else from the road class, per direction |

## MATSim

| Your tool | duckOSM |
|---|---|
| pt2matsim `Osm2MultimodalNetwork` | `net.driving.to_matsim("network.xml.gz")`, or `duckosm.sim.to_matsim(net, ..., mode=["driving", "cycling"])` |
| MATSim lanes and signals | `duckosm.sim.to_matsim_lanes(gmns_db, "out/matsim")` |
| link volumes by link id | `duckosm.sim.edge_map("network.xml.gz")`: the link id is the `edge_id` |

## Join the results back

```python
files = net.driving.to_sumo("out/sumo", run_netconvert=True)
ids = duckosm.sim.edge_map(files)                                   # SUMO id -> edge_id
counts = pd.read_xml("out/sumo/edgedata.xml", xpath=".//edge", parser="etree", dtype={"id": str})
cars = counts.merge(ids, on="id").groupby("edge_id").entered.sum()
net.driving.map(color_by=cars)
```
