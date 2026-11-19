# SUMO

`duckosm sumo` writes a network for [SUMO](https://eclipse.dev/sumo/), with your OSM fixes. Each SUMO edge id is the
duckOSM `edge_id`.

| Reference and format | |
|---|---|
| Format | SUMO [plain XML](https://sumo.dlr.de/docs/Networks/PlainXML.html) (`.nod.xml`, `.edg.xml`, `.con.xml`), and the [SUMO network](https://sumo.dlr.de/docs/Networks/SUMO_Road_Networks.html) `.net.xml` that netconvert makes from it |
| Reference tool | SUMO's [netconvert](https://sumo.dlr.de/docs/netconvert.html) |
| Check | netconvert reads the plain XML and keeps each edge id and each connection. [How each output is checked](../concepts/checks.md) |

```bash
pip install "duckosm[sumo]"                      # adds SUMO's netconvert
duckosm sumo monaco.duckdb                       # -> sumo/monaco.net.xml
duckosm sumo monaco.duckdb --no-netconvert       # only the plain XML, as input for your own netconvert
duckosm sumo monaco.duckdb --join-junctions      # netconvert joins the junctions that are near each other
```

## What goes in

| SUMO item | From duckOSM |
|---|---|
| edge id | `edge_id` |
| connections | the legal turns (`edge_graph`), so the turn restrictions stay. netconvert adds no turn |
| lanes | one SUMO lane for each lane, with its width and its vehicle classes, numbered from the right |
| lane classes | bus lane `bus`, bike lane `bicycle`, car lane: each road vehicle |
| walking network | one 2 m lane for pedestrians, and SUMO walking areas at the junctions |
| cycling network | one 1.5 m lane for bicycles |

The coordinates are in metres, in the UTM zone of the data.

## See it

`duckosm sumo-map sumo/monaco.net.xml` draws each SUMO lane at its width. The colors are those of
[`gmns-map`](gmns.md#see-it): a car lane is gray, a bus lane red, a bike lane blue.

<figure class="pair" markdown="span">
  ![The lanes of one Monaco junction from the GMNS export, drawn by roadstyle](../images/tour-lanes-gmns.png)
  ![The same junction from the SUMO export after netconvert, drawn by roadstyle](../images/tour-lanes-sumo.png)
  <figcaption>One Monaco junction: the GMNS lanes (left, <code>duckosm gmns-map</code>) and SUMO's lanes after netconvert (right, <code>duckosm sumo-map</code>).</figcaption>
</figure>

| | GMNS | SUMO |
|---|---|---|
| Lane shape | duckOSM: the road line, moved to the side for each lane | netconvert: each lane ends at the edge of its junction |
| Inside a junction | the paths of SUMO, kept as `lane_connector` | netconvert's junction shape and its internal lanes |
| Lane ids | `<link_id>_<lane_num>`, lane 1 on the left | `<edge_id>_<index>`, index 0 on the right |
| Tools | DuckDB, Path4GMNS, DTALite | `sumo`, `sumo-gui`, `netedit` |

## Speed and memory

Mac mini (Apple M6, 12 cores, 16 GB). The plain XML only (`--no-netconvert`).

| Area (input) | Time | Memory | Output |
|---|---|---|---|
| Kentucky (148 MB) | 11 s | 5.0 GB | 1.4 GB |
| France (5.1 GB) | not run: the Mac mini has too little free disk | | |

## duckOSM and SUMO's OSM import

SUMO can also read OSM itself: `netconvert --osm-files`. The same area, both ways, on the Mac mini:

| | duckOSM (`duckosm sumo`) | SUMO's OSM import |
|---|---|---|
| Time, memory: Monaco | measuring | measuring |
| Time, memory: Kentucky | measuring | measuring |
| Edges, lanes, connections | measuring | measuring |
| Turn restrictions kept | all (from `edge_graph`) | measuring |
| OSM fixes | yes | no |
| Edge ids | `edge_id`, the same after each build | OSM way id and a part number |

## Limits

- No painted turn arrows (`turn:lanes`): SUMO's plain XML has no field for them. The [GMNS](gmns.md) export has them.
- No demand, routes or `.sumocfg`: these are your scenario.

All options: [Command line](../reference/cli.md#sumo).
