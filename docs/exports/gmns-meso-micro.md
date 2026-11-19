# GMNS meso and micro networks

`duckosm gmns --meso --micro` adds two networks for simulators that work below the road: road sections (meso) and
lane cells (micro), with your OSM fixes. The ids come from `edge_id`, so they stay the same after each build.

The three networks show the same roads at three levels of detail:

| Network | One link is | For |
|---|---|---|
| macro (`duckosm gmns`) | one direction of a road, from junction to junction, with its lanes as attributes | traffic assignment and routing: DTALite, Path4GMNS |
| meso (`--meso`) | a road section, and a connector for each legal turn through a junction | simulators that move groups of vehicles along roads and through junctions |
| micro (`--micro`) | a cell of about 7 m of one lane, a lane change, or a lane-to-lane turn | simulators that move each vehicle cell by cell and lane by lane |

| Reference and format | |
|---|---|
| Format | tables in the GMNS DuckDB file, in the layout of the meso and micro networks of [osm2gmns](https://github.com/jiawlu/OSM2GMNS). [GMNS](https://github.com/zephyr-data-specs/GMNS) has no meso or micro tables |
| Reference tool | [osm2gmns](https://github.com/jiawlu/OSM2GMNS) 0.7.6, compared by hand |
| Check | the tests check the lengths, the link ends and the number of cells. [How each output is checked](../concepts/checks.md) |

```bash
duckosm gmns monaco.duckdb --meso --micro                  # -> monaco_gmns.duckdb with meso_driving, micro_driving
duckosm gmns monaco.duckdb --meso --meso-mode cycling      # the meso network of cycling
```

## The tables

| Network | Table | One row is |
|---|---|---|
| meso | `meso_<mode>.meso_node` | the end of a road section |
| | `meso_<mode>.meso_link` | a road section, about 6 m shorter than its road at each end, or a connector: a curve for one legal turn between two sections |
| micro | `micro_<mode>.micro_node` | the start of a cell of a lane |
| | `micro_<mode>.micro_link` | a cell of about 7 m, a lane change between cells side by side, or a turn through a junction |

| Table | Columns |
|---|---|
| `meso_node` | `node_id` (`<link_id>u` start, `<link_id>d` end), `x_coord`, `y_coord`, `macro_node_id`, `macro_link_id`, `geom` |
| `meso_link` | `link_id` (`M<link_id>` a section, `X<ib>-<ob>` a connector), `meso_type` (`normal`, `movement`), `from_node_id`, `to_node_id`, `length`, `lanes`, `free_speed`, `facility_type`, `macro_link_id`, `movement_id`, `start_ib_lane`, `end_ib_lane`, `geometry`, `geom` |
| `micro_node` | `node_id` (`<lane_id>@<k>`), `x_coord`, `y_coord`, `lane_id`, `cell_k`, `macro_link_id`, `lane_no`, `geom` |
| `micro_link` | `link_id` (`C…` a cell, `H…` a lane change, `X…` a turn), `cell_type` (`normal`, `lane_change`, `movement`), `from_node_id`, `to_node_id`, `length`, `width`, `lane_no`, `macro_link_id`, `meso_link_id`, `geometry`, `geom` |

Lengths are in metres, and each link starts and ends at its nodes. `duckosm gmns-viz monaco_gmns.duckdb` draws the meso
and micro networks.

## Speed and memory

Mac mini (Apple M6, 12 cores, 16 GB).

| Area (input) | Time | Memory | Output |
|---|---|---|---|
| Monaco (0.7 MB) | 30 s | 148 MB | 18.9 MB |
| Kentucky (148 MB) | stops: netconvert rejects the connection file (28 unknown edges) | | |
| France (5.1 GB) | not run | | |

## duckOSM and osm2gmns

| | duckOSM | osm2gmns 0.7.6 |
|---|---|---|
| Ids | from `edge_id`, the same after each build | numbered on each run |
| Road class | the OSM `highway` in `facility_type` | a number in `link_type`, a name in `link_type_name` |
| Junctions | as in OSM | merged, so the network is coarser |
| Columns that only one has | `geom`, `meso_type`, the lane ranges, `width`, `cell_k` | `zone_id`, `activity_type`, `is_boundary`, a micro `capacity` |

All options: [Command line](../reference/cli.md#gmns).
