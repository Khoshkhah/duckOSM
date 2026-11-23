# GMNS

`duckosm gmns` writes the network in [GMNS](https://zephyr-data-specs.github.io/GMNS/), the table format of DTALite,
Path4GMNS and other network models, with your OSM fixes. Each GMNS `link_id` is the duckOSM `edge_id`. In driving,
the links are the edges of `driving.edges` and also the bus-only edges of `driving.private_edges` (`access = 'bus'`:
a bus lane against a one-way street, or a bus-only road). duckOSM does not route on these edges, so they are not in
`driving.edges`. In GMNS they have `allowed_uses` `bus` (or `bus,bike`) and one lane. On Monaco, the 18 bus-only edges
give 18 links and 4 nodes more than `driving.edges` and `driving.nodes`.

| Reference and format | |
|---|---|
| Format | [GMNS](https://github.com/zephyr-data-specs/GMNS) 0.97: all tables in one DuckDB file, and with `--to-csv` the CSV files of the specification |
| Reference tool | [frictionless](https://framework.frictionlessdata.io/) (the schemas) and [Path4GMNS](https://github.com/jdlph/Path4GMNS) (a route on the network) |
| Check | the CSV files pass the schemas of the specification, and Path4GMNS loads them and finds a route. `--check` checks the values: link ends, lengths, lane numbers, movements. [How each output is checked](../concepts/checks.md) |

```bash
pip install "duckosm[sumo]"                      # optional: without netconvert, the movements have no lanes
duckosm gmns monaco.duckdb                       # -> monaco_gmns.duckdb
duckosm gmns monaco.duckdb --to-csv gmns/        # also the CSV files
```

## Macro, meso and micro

`duckosm gmns` makes the macro network. `--meso` and `--micro` add two more detailed networks of the same roads:

| Network | One link is | For |
|---|---|---|
| macro (`duckosm gmns`) | one direction of a road, from junction to junction, with its lanes as attributes | traffic assignment and routing: DTALite, Path4GMNS |
| meso (`--meso`) | a road section, and a connector for each legal turn through a junction | simulators that move groups of vehicles along roads and through junctions |
| micro (`--micro`) | a cell of about 7 m of one lane, a lane change, or a lane-to-lane turn | simulators that move each vehicle cell by cell and lane by lane |

The meso and micro tables: [GMNS meso and micro](gmns-meso-micro.md).

## The tables

One schema for each mode: `gmns_driving`, `gmns_walking`, `gmns_cycling`. The CSV files have only the columns of the
specification. The geometries and the lane connectors stay in the DuckDB file.

| Table | Holds |
|---|---|
| [`node`](gmns_tables.md#node) | the junctions. `ctrl_type = 'signal'` at traffic lights |
| [`link`](gmns_tables.md#link) | one row for each direction of a road: length, speed, lanes, capacity, `facility_type` (the OSM `highway`), name |
| [`geometry`](gmns_tables.md#geometry) | the shape of each link |
| [`lane`](gmns_tables.md#lane) | one row for each lane: its use, width and turns |
| [`movement`](gmns_tables.md#movement) | the legal turns, and the lanes that each turn starts from and ends in |
| [`location`](gmns_tables.md#location) | OSM points on a link: crossings, stop signs, signals, parking entrances |
| [`signal_controller`](gmns_tables.md#signal_controller), [`curb_seg`](gmns_tables.md#curb_seg), [`zone`](gmns_tables.md#zone), [`config`](gmns_tables.md#config), [`use_definition`](gmns_tables.md#use_definition), [`use_group`](gmns_tables.md#use_group) | the signals, parking, the area, the units and the modes |
| [`lane_connector`](gmns_tables.md#lane_connector), [`crossing`](gmns_tables.md#crossing), [`lane_crossing`](gmns_tables.md#lane_crossing), [`link_along`](gmns_tables.md#link_along), [`lane_check`](gmns_tables.md#lane_check) | duckOSM tables: the paths through a junction, the pedestrian crossings, and the lanes that got no connection |

Click a table for its columns, in the Reference: [GMNS tables](gmns_tables.md).

![One junction in GMNS: a node, a link with two lanes, and the movements each lane can take](../images/gmns_junction.svg)

## Lanes from OSM tags

| OSM tag | Gives |
|---|---|
| `lanes`, `lanes:forward`, `lanes:backward` | the number of lanes in each direction |
| `turn:lanes` | the turns of each lane |
| `width:lanes` | the width of each lane |
| `bicycle:lanes`, `psv:lanes` | bike lanes and bus lanes |

Most ways have none of these tags, so most lanes are general lanes with a default width.

## See it

`duckosm gmns-map monaco_gmns.duckdb --source-db monaco.duckdb` draws each lane at its width on a base map. Click a lane:
it turns red, and the lanes that its turns lead into turn green.

<iframe src="../../maps/monaco_gmns_lanes.html" title="Monaco's lanes, drawn by duckosm gmns-map"
        loading="lazy" style="width: 100%; height: 560px; border: 0; border-radius: 8px"></iframe>

## Speed and memory

An area with nodes in two or more map tiles (zoom 8, the tiles of `duckosm tiles`; `--tile-zoom`) is made one tile at a time, with a band of 2 km
around the tile. The memory is that of the largest tile, not of the area
([gmns_by_tiles](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/gmns_by_tiles.md)). Driving, with
netconvert. Memory: the program and netconvert together.

| Area (input) | Machine | Time | Memory | Output |
|---|---|---|---|---|
| Monaco (0.7 MB), all modes | Mac mini (M6, 16 GB) | 0.79 s | 145 MB | 8.4 MB |
| Kentucky (148 MB), whole area (before) | Mac mini (M6, 16 GB) | 1085 s | 25.9 GB | 6.2 GB |
| Kentucky (148 MB), 18 tiles (zoom 8, the default) | WSL (20 cores, 23 GB) | 1828 s | 9.0 GB | 5.6 GB |
| Kentucky (148 MB), 49 tiles (zoom 9) | WSL (20 cores, 23 GB) | 1406 s | 5.0 GB | 5.9 GB |
| Île-de-France (duckOSM db 2.2 GB), 15 tiles (zoom 9) | WSL (20 cores, 23 GB) | 1373 s | 8.5 GB | 2.6 GB |
| France (5.1 GB) | | not run: the GMNS db (about 150 GB) does not fit on the disk | | |

Île-de-France stands for the densest part of France: Paris is one tile (9/259/176, 734 s, 1.3 million movements).

Kentucky by tiles takes more time than the whole area on the Mac mini (1406 s against 1085 s):

| Cause | Share |
|---|---|
| Other machine: the whole area with netconvert needs about 26 GB, so it does not run on WSL | not known |
| The copy of the source and the merge of each tile | 145 s (10 %) |
| The GMNS steps of the 49 tiles, with netconvert on each tile. The roads of the band of 2 km are done in two tiles or more | 1259 s; the part of the band is not measured |

Zoom 8 uses more memory than zoom 9 (Kentucky: 9.0 GB against 5.0 GB), because a tile is larger. It is the default
because it is the tiling of `duckosm tiles`: all steps use one tiling. `--tile-zoom 9` or `10` gives less memory.

On the GMNS db of Kentucky (zoom 8, WSL): `duckosm lanelet2` 175 s, 8.9 GB, 9.8 GB file; `duckosm opendrive
--junctions` 64 s, 2.1 GB, 8.0 GB file.

Known limit: the netconvert of the eclipse-sumo package for macOS (1.27 and 1.28, Apple silicon) stops with a crash
(exit 245) in "Computing node shapes" on large tiles of Kentucky (zoom 8 and 9). netconvert 1.27 on Linux builds the
same input files. netconvert 1.28 on Linux also stops on the Kentucky tile 9/134/197: on Linux, use SUMO 1.27
(`pip install "eclipse-sumo==1.27.*" "sumo-data==1.27.*"`).

## duckOSM and osm2gmns

[osm2gmns](https://github.com/jiawlu/OSM2GMNS) is the most used OSM-to-GMNS tool.

| | duckOSM | osm2gmns 0.7 |
|---|---|---|
| Ids | `edge_id`, the same after each build | numbered from 0 on each run |
| OSM fixes | yes | no |
| Lanes | each lane with its shape, and from the OSM tags its turns, its use (bus, bike) and its width | each lane with its shape |
| Turns | the legal turns, with the OSM turn restrictions | from the geometry |
| Complex junctions | as in OSM | merged into one |
| Micro network | [meso and micro](gmns-meso-micro.md), new | mature |

All options: [Command line](../reference/cli.md#gmns). The design and the audit against the specification:
[gmns_spec_conformance](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/gmns_spec_conformance.md).
