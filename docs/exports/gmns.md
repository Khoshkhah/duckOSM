# GMNS

Part of **[3. GMNS network](gmns-network/index.md)**.

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
pip install "duckosm[sumo]"                      # optional: without netconvert, movements have no lanes

# Input: Level 2 routable database (monaco.duckdb)
# Output: Level 3 GMNS database (monaco_gmns.duckdb by default, or with -o)
duckosm gmns monaco.duckdb                       # -> creates monaco_gmns.duckdb
duckosm gmns monaco.duckdb -o out/gmns.duckdb    # custom output file path
duckosm gmns monaco.duckdb --to-csv gmns/        # also exports standard GMNS CSVs into gmns/
```

## Macro, meso and micro

`duckosm gmns` makes the macro network. `--meso` and `--micro` add two more detailed networks of the same roads:

| Network | One link is | For |
|---|---|---|
| macro (`duckosm gmns`) | one direction of a road, from junction to junction, with its lanes as attributes | traffic assignment and routing: DTALite, Path4GMNS, PyG HeteroData |
| meso (`--meso`) | a road section, and a connector for each legal turn through a junction | simulators that move groups of vehicles along roads and through junctions |
| micro (`--micro`) | a cell of about 7 m of one lane, a lane change, or a lane-to-lane turn | simulators that move each vehicle cell by cell and lane by lane |

```bash
duckosm gmns monaco.duckdb --meso --micro                  # creates monaco_gmns.duckdb with meso_driving & micro_driving
duckosm gmns monaco.duckdb -o out/gmns.duckdb --meso      # writes to custom output path
duckosm gmns-viz monaco_gmns.duckdb                        # draws the meso and micro networks in an interactive HTML viewer
```

## Network schemas

In `<area>_gmns.duckdb`, **every table belongs to a network schema** (the default `main` schema is empty). GMNS is dedicated purely to transport networks, unlike Level 2 (`<area>.duckdb`) which also stores GIS layers (`features`) and raw OSM tags (`raw`).

### Macro schemas (`duckosm gmns`)

* **`gmns_driving` (16 tables):** The vehicular road network for cars, trucks, and buses. Models lanes (`lane`), lane-level turns with restrictions (`movement`), signals (`signal_controller`), on-street parking (`curb_seg`), crosswalks spanning road lanes (`crossing`, `lane_crossing`), and smooth intersection curves (`lane_connector`).
* **`gmns_walking` (14 tables):** Sidewalks, footpaths, stairs, plazas, and crosswalks. Tracks parallel roadways via `link_along` and connects footways to road crossings via `lane_connector`.
* **`gmns_cycling` (13 tables):** Cycleways, protected bike paths, shared lanes, and bikeable roads. Tracks parallel roadways via `link_along`.
* **`gmns_all` (5 tables, via `--combined`):** Merges shared physical streets across driving, walking, and cycling into a single link with unioned `allowed_uses = 'auto,bike,walk'` (`link_id` = `edge_id`). Lanes and turns stay in the mode schemas.

### Meso and micro schemas (`--meso`, `--micro`)

* **`meso_<mode>` (2 tables, via `--meso`):** Road sections (`meso_link`, trimmed ~6 m from junctions) and junction turn connectors between section endpoints (`meso_node`). Default: `meso_driving`.
* **`micro_<mode>` (2 tables, via `--micro`):** Discrete 7-meter lane cells, lateral lane changes, and turn movements (`micro_link`) between cell boundary vertices (`micro_node`). Default: `micro_driving`.

* **In DuckDB (`.duckdb`):** Stores all tables with native DuckDB spatial geometries (`geom`) for fast spatial SQL queries and map rendering.
* **In CSV (`--to-csv`):** Exports the standard specification tables with WKT text geometries (`geometry`), ready for DTALite and Path4GMNS.

See the complete table matrix and column definitions in **[GMNS tables](gmns_tables.md)**.

![One junction in GMNS: a node, a link with two lanes, and the movements each lane can take](../images/gmns_junction.svg)

## Meso and micro networks

`duckosm gmns --meso --micro` adds two networks for simulators that work below the road: road sections (meso) and lane cells (micro), with your OSM fixes. The ids come from `edge_id`, so they stay the same after each build.

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

Lengths are in metres, and each link starts and ends at its nodes.

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

Driving network, with netconvert. Input is the Level 2 routable database (`<area>.duckdb`):

| Area (input DuckDB) | Machine | Time | Memory | Output GMNS DuckDB |
|---|---|---|---|---|
| Monaco (38 MB `.duckdb`) | Mac mini (M6, 16 GB) | 0.8 s | 145 MB | 8.4 MB |
| Kentucky (~1.2 GB `.duckdb`), 49 tiles | WSL (20 cores, 23 GB) | 23 min | 5.0 GB | 5.9 GB |
| Île-de-France (2.2 GB `.duckdb`), 15 tiles | WSL (20 cores, 23 GB) | 23 min | 8.5 GB | 2.6 GB |
| France (~35 GB `.duckdb`) | | not run | (estimated ~150 GB) | |

Large areas run tile-by-tile (`--tile-zoom 9`), keeping peak memory bounded by the tile rather than the whole region.

## duckOSM and osm2gmns

[osm2gmns](https://github.com/jiawlu/OSM2GMNS) is the most used OSM-to-GMNS tool.

| | duckOSM | osm2gmns 0.7 |
|---|---|---|
| Ids | `edge_id`, the same after each build | numbered from 0 on each run |
| OSM fixes | yes | no |
| Lanes | each lane with its shape, and from the OSM tags its turns, its use (bus, bike) and its width | each lane with its shape |
| Turns | the legal turns, with the OSM turn restrictions | from the geometry |
| Complex junctions | as in OSM | merged into one |
| Meso and micro | included, with stable `edge_id` | mature |
| Columns that only one has | `geom`, `meso_type`, the lane ranges, `width`, `cell_k` | `zone_id`, `activity_type`, `is_boundary`, a micro `capacity` |

All options: [Command line](../reference/cli.md#gmns).
