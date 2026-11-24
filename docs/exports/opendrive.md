# OpenDRIVE (experimental)

`duckosm opendrive` writes an ASAM OpenDRIVE 1.7 `.xodr` file, with your OSM fixes, for driving simulators (CARLA,
esmini) and microsimulators (PTV Vissim, Aimsun). Each OpenDRIVE road id is the duckOSM `edge_id`.

| Reference and format | |
|---|---|
| Format | [ASAM OpenDRIVE 1.7](https://www.asam.net/standards/detail/opendrive/) (`.xodr`) |
| Reference tool | none: no test loads the file in CARLA or [esmini](https://github.com/esmini/esmini) yet |
| Check | the tests check the XML structure and the order of its elements. [How each output is checked](../concepts/checks.md) |

```bash
duckosm opendrive monaco.duckdb                       # roads only
duckosm gmns monaco.duckdb                            # for the junctions: a GMNS database first
duckosm opendrive monaco_gmns.duckdb --junctions      # roads, connecting roads and junctions
```

Without `--junctions`, the roads do not link: you can look at them, but a vehicle cannot drive from road to road. With
`--junctions`, each legal turn becomes a connecting road, so vehicles can drive through the junctions.

## What goes in

| OpenDRIVE item | From duckOSM |
|---|---|
| `road` | one direction of an edge. `id` is the `edge_id` |
| reference line | the edge line, in metres, in the UTM zone of the data (or `--crs`) |
| `lane` | the driving lanes, each 3.25 m wide |
| elevation profile | the heights of [`duckosm elevation`](../guides/elevation.md). Not with `--junctions` |
| connecting road | one legal turn through a junction, as a smooth curve (`--junctions`) |
| `junction` | the connecting roads of one junction (`--junctions`) |

## Speed and memory

The file is written while DuckDB reads the rows. The memory stays near DuckDB's limit of 2 GB and does not grow with
the area.

| Area (input DuckDB) | Machine | Command | Time | Memory | Output |
|---|---|---|---|---|---|
| Monaco (38 MB `.duckdb`) | Mac mini (M6, 16 GB) | `--junctions` | 0.07 s | 0.04 GB | 5.7 MB |
| Kentucky (~1.2 GB `.duckdb`) | WSL (20 cores, 23 GB) | roads only | 21 s | 0.52 GB | 2.3 GB |
| Kentucky (~1.2 GB `.duckdb`) | Mac mini (M6, 16 GB) | `--junctions` ¹ | 47 s | 2.0 GB | 7.9 GB |
| Île-de-France (2.2 GB `.duckdb`) ² | WSL (20 cores, 23 GB) | roads only | 8.4 s | 0.26 GB | 1.2 GB |
| Île-de-France (2.2 GB `.duckdb`) ² | WSL (20 cores, 23 GB) | `--junctions`, GMNS with netconvert | 32 s | 2.1 GB | 5.1 GB |
| France (~35 GB `.duckdb`) | | | not run: its GMNS db does not fit on the disk | | |

¹ On the GMNS db of `duckosm gmns` with the default options (the lane rules). With `--lane-source netconvert`, a turn can
have one movement for each run of lanes, so the file has more connecting roads.

² Île-de-France stands for the densest part of France (Paris).

## Limits

| Limit | Why |
|---|---|
| Each lane is 3.25 m wide | OSM `width:lanes` is not read yet |
| A connecting road has one lane, joined to the rightmost lane | not made yet |
| A connecting road ends about 2 m from its road, sometimes more. A strict simulator can report it | not fixed yet |
| No road type, speed limit, markings, signals or banking | not made yet |
