# 3. Simulation network

The network in the formats of traffic simulation tools, from macro (links) to micro (lanes and cells).

| Tool | Output | Command |
|---|---|---|
| DTALite, Path4GMNS | [GMNS](../gmns.md): the macro tables ([all GMNS tables](../gmns_tables.md)) | `duckosm gmns` |
| mesoscopic and microscopic simulators | [GMNS meso and micro](../gmns-meso-micro.md) | `duckosm gmns --meso --micro` |
| MATSim, BEAM, eqasim | [MATSim](../matsim.md): `network.xml`, lanes and signals | `duckosm matsim`, `matsim-lanes` |
| SUMO | [SUMO](../sumo.md): plain XML and `.netccfg` (netconvert makes the `.net.xml`) | `duckosm sumo` |
| OpenTrack, RailSys (rail planning) | [railML 2.4](../railml.md) | `duckosm railml` |

railML is **experimental**. Tests check its structure. It is not loaded in the target tools yet.
